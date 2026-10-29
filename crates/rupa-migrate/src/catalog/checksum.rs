use sha2::{Digest, Sha256};

use super::{CHECKSUM_FORMAT, CatalogEntry, Checksum, ValidationError, encode, normalize};
use crate::{MigrationKind, Operation, Plan, Policy, Reversal, VersionScheme};

/// The artifact checksum. A planned entry's identity is its plan plus any
/// declared inputs (an SQL file's source); a planned Rust migration declares
/// none, so source refactors that keep its plan keep its checksum. An
/// imperative body is opaque, so its declared source closure is required. A
/// down body is appended last and only when present, so an irreversible
/// artifact keeps the checksum it had before down bodies existed. A
/// snapshot's coverage manifest follows its inputs.
pub(super) fn checksum(
    stream: &str,
    scheme: VersionScheme,
    entry: &CatalogEntry,
) -> Result<Checksum, ValidationError> {
    let CatalogEntry {
        metadata,
        plan,
        reversal,
        covers,
        ..
    } = entry;
    let plan = plan.as_ref();
    let mut hash = Sha256::new();
    let kind = match metadata.kind {
        MigrationKind::Incremental => "incremental",
        MigrationKind::Snapshot => "snapshot",
    };
    let policy = match metadata.policy {
        Policy::TransactionalRequired => "transactional-required",
        Policy::NonTransactional => "nontransactional",
    };
    let empty = |plan: &Plan| plan.operations().is_empty();

    if metadata.inputs.is_empty() && plan.is_none() {
        return Err(ValidationError::NoInputs(metadata.version.into()));
    }

    if plan.is_some_and(empty) || matches!(reversal, Reversal::Planned(down) if empty(down)) {
        return Err(ValidationError::EmptyPlan(metadata.version.into()));
    }

    for field in [
        CHECKSUM_FORMAT,
        stream,
        scheme.as_str(),
        kind,
        metadata.version,
        "sync",
        policy,
    ] {
        hash_field(&mut hash, field.as_bytes());
    }

    match plan {
        Some(plan) => {
            hash_field(&mut hash, b"planned");
            hash_plan(&mut hash, plan)?;
        }
        None => hash_field(&mut hash, b"imperative"),
    }

    hash_inputs(&mut hash, metadata.inputs)?;

    if metadata.kind == MigrationKind::Snapshot {
        hash_field(&mut hash, b"covers");
        hash_field(&mut hash, &(covers.len() as u64).to_be_bytes());

        for covered in covers {
            hash_field(&mut hash, covered.version.to_string().as_bytes());
            hash_field(&mut hash, covered.checksum.as_bytes());
        }
    }

    match reversal {
        Reversal::Irreversible => {}
        Reversal::Planned(down) => {
            hash_field(&mut hash, b"down-planned");
            hash_plan(&mut hash, down)?;
        }
        Reversal::Imperative => hash_field(&mut hash, b"down-imperative"),
    }

    Ok(Checksum(hash.finalize().into()))
}

/// Hashes a plan's operation count and each operation.
fn hash_plan(hash: &mut Sha256, plan: &Plan) -> Result<(), ValidationError> {
    let operations = plan.operations();

    hash_field(hash, &(operations.len() as u64).to_be_bytes());

    for operation in operations {
        match operation {
            Operation::Sql(sql) => {
                let sql = normalize(sql, "planned SQL")?;

                hash_field(hash, b"sql");
                hash_field(hash, sql.as_bytes());
            }
            Operation::Schema(op) => {
                hash_field(hash, b"schema");
                encode::Encoder(hash).schema(op);
            }
        }
    }

    Ok(())
}

/// Hashes the declared inputs in path order, rejecting invalid or duplicate paths.
fn hash_inputs(hash: &mut Sha256, inputs: &[crate::ArtifactInput]) -> Result<(), ValidationError> {
    let mut inputs: Vec<_> = inputs.iter().collect();

    hash_field(hash, b"inputs");
    hash_field(hash, &(inputs.len() as u64).to_be_bytes());
    inputs.sort_by_key(|input| input.path);

    for (index, input) in inputs.iter().enumerate() {
        if input.path.is_empty()
            || input.path.contains('\\')
            || input
                .path
                .split('/')
                .any(|c| c.is_empty() || c == "." || c == ".." || c.contains(':'))
            || (index > 0 && inputs[index - 1].path == input.path)
        {
            return Err(ValidationError::InputPath(input.path.into()));
        }

        let contents = normalize(input.contents, input.path)?;

        hash_field(hash, input.path.as_bytes());
        hash_field(hash, contents.as_bytes());
    }

    Ok(())
}

pub(super) fn hash_field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
