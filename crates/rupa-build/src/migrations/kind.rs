use syn::{Item, Type};

use super::Error;

const PLANNED: &str = "PlannedMigration";
const SCRIPTED: &str = "Migration";

/// Whether the migration root's type `name` is planned: it implements
/// `PlannedMigration` at the root's top level. Planned migrations are
/// identified by their plan, so they need neither a hashed source closure
/// nor isolation from shared application code.
pub(super) fn planned(logical: &str, source: &str, name: &str) -> Result<bool, Error> {
    let file = syn::parse_file(source).map_err(|e| Error::Parse {
        path: logical.to_owned(),
        message: e.to_string(),
    })?;
    let mut planned = false;
    let mut scripted = false;

    for item in &file.items {
        let Item::Impl(item) = item else {
            continue;
        };
        let Some((_, path, _)) = &item.trait_ else {
            continue;
        };
        let Type::Path(self_ty) = item.self_ty.as_ref() else {
            continue;
        };

        if !self_ty
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == name)
        {
            continue;
        }

        match path.segments.last().map(|s| s.ident.to_string()).as_deref() {
            Some(PLANNED) => planned = true,
            Some(SCRIPTED) => scripted = true,
            _ => {}
        }
    }

    if planned && scripted {
        return Err(Error::MixedKinds(logical.to_owned()));
    }

    Ok(planned)
}
