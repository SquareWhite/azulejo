//! Stable IDs. Each is a ULID with a kind prefix, e.g. `tile_01JBX3…`, so an ID
//! pasted into the wrong command fails to parse instead of matching nothing.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// Returned when a string isn't a valid ID of the expected kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIdError {
    pub expected_prefix: &'static str,
    pub input: String,
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a valid {}_… ID",
            self.input, self.expected_prefix
        )
    }
}

impl std::error::Error for ParseIdError {}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Ulid);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// A fresh, random ID.
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Ulid::generate())
            }

            pub fn ulid(self) -> Ulid {
                self.0
            }
        }

        impl From<Ulid> for $name {
            fn from(ulid: Ulid) -> Self {
                Self(ulid)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}_{}", $prefix, self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let err = || ParseIdError {
                    expected_prefix: $prefix,
                    input: s.to_owned(),
                };
                let rest = s
                    .strip_prefix($prefix)
                    .and_then(|r| r.strip_prefix('_'))
                    .ok_or_else(err)?;
                Ulid::from_string(rest).map(Self).map_err(|_| err())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = Cow::<str>::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({
                    "type": "string",
                    "pattern": concat!("^", $prefix, "_[0-9A-HJKMNP-TV-Z]{26}$"),
                })
            }
        }
    };
}

id_type!(
    /// An app window.
    WindowId,
    "win"
);
id_type!(
    /// A workspace; maps one-to-one to a herdr workspace.
    WorkspaceId,
    "ws"
);
id_type!(
    /// A tab inside a workspace.
    TabId,
    "tab"
);
id_type!(
    /// A tile: one leaf of a tab's layout tree.
    TileId,
    "tile"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_strings_and_json() {
        let id = TileId::new();
        let text = id.to_string();
        assert!(text.starts_with("tile_"));
        assert_eq!(text.parse::<TileId>().unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<TileId>(&json).unwrap(), id);
    }

    #[test]
    fn wrong_prefix_is_rejected() {
        let tab = TabId::new().to_string();
        assert!(tab.parse::<TileId>().is_err());
        assert!("tile_nope".parse::<TileId>().is_err());
    }
}
