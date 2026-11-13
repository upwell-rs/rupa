//! Canonical records for column kinds and index methods: their whole
//! rendering configuration.

use rupa_core::SqlType;
use rupa_core::schema::{ColumnSpec, IndexMethod, IndexSupport, TypeSupport};

use super::Encoder;

impl Encoder<'_> {
    /// A kind's rendering configuration, so changing any of it changes the
    /// checksum. The reason a dialect lacks a type is a diagnostic: rewording
    /// it does not change what is applied.
    pub(super) fn spec(&mut self, spec: ColumnSpec) {
        self.str(spec.name);

        for support in [spec.postgres, spec.mysql, spec.sqlite] {
            match support {
                TypeSupport::Named { name, args } => {
                    self.str("named");
                    self.str(name);
                    self.len(args.len());

                    for arg in args {
                        self.len(*arg as usize);
                    }
                }
                TypeSupport::Unsupported(_) => self.str("unsupported"),
            }
        }

        self.str(sql_type_name(spec.sql_type));
        self.flag(spec.bounded);
    }

    pub(super) fn method(&mut self, kind: IndexMethod) {
        self.str("using");
        self.str(kind.name);

        for support in [kind.postgres, kind.mysql, kind.sqlite] {
            match support {
                IndexSupport::Default => self.str("default"),
                IndexSupport::Using(method) => {
                    self.str("using-method");
                    self.str(method);
                }
                IndexSupport::Unsupported(_) => self.str("unsupported"),
            }
        }

        self.flag(kind.unique);
        self.flag(kind.multi_column);
    }
}

pub(super) fn sql_type_name(ty: SqlType) -> &'static str {
    match ty {
        SqlType::Bool => "bool",
        SqlType::I16 => "i16",
        SqlType::I32 => "i32",
        SqlType::I64 => "i64",
        SqlType::F32 => "f32",
        SqlType::F64 => "f64",
        SqlType::Text => "text",
        SqlType::Bytes => "bytes",
        SqlType::Uuid => "uuid",
        SqlType::Date => "date",
        SqlType::Time => "time",
        SqlType::Timestamp => "timestamp",
        SqlType::TimestampTz => "timestamptz",
        SqlType::Json => "json",
    }
}
