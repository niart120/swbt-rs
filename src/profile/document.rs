use std::{fmt, marker::PhantomData};

use serde_json::Value;

use crate::{
    error::{Error, ErrorKind},
    model::ControllerModel,
};

use super::ControllerKind;

const PROFILE_FORMAT: &str = "swbt.profile";
const PROFILE_SCHEMA_VERSION: u64 = 2;

pub(crate) struct ProfileDocument {
    controller_kind: ControllerKind,
    value: Value,
}

impl ProfileDocument {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T29 parses an existing profile before controller construction"
        )
    )]
    pub(crate) fn parse_json(bytes: &[u8]) -> crate::Result<Self> {
        let value = serde_json::from_slice::<Value>(bytes).map_err(|source| {
            Error::with_source(
                ErrorKind::InvalidProfile,
                "profile document is not valid JSON",
                source,
            )
        })?;

        let controller_kind = {
            let object = value.as_object().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidProfile,
                    "profile document must be a JSON object",
                )
            })?;

            if object.get("format").and_then(Value::as_str) != Some(PROFILE_FORMAT) {
                return Err(Error::new(
                    ErrorKind::InvalidProfile,
                    "profile format must be swbt.profile",
                ));
            }
            if object.get("schema_version").and_then(Value::as_u64) != Some(PROFILE_SCHEMA_VERSION)
            {
                return Err(Error::new(
                    ErrorKind::InvalidProfile,
                    "profile schema version must be 2",
                ));
            }

            let kind_name = object
                .get("controller_kind")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidProfile,
                        "profile controller_kind must be a supported string",
                    )
                })?;
            let controller_kind = ControllerKind::ALL
                .iter()
                .copied()
                .find(|kind| kind.profile_name() == kind_name)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidProfile,
                        "profile controller_kind must be a supported string",
                    )
                })?;

            let identity = object
                .get("identity")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidProfile,
                        "profile identity must be a JSON object",
                    )
                })?;
            match identity.get("kind").and_then(Value::as_str) {
                Some("adapter-default") if !identity.contains_key("address") => {}
                Some("exp-local-address")
                    if identity.get("address").is_some_and(Value::is_string) => {}
                _ => {
                    return Err(Error::new(
                        ErrorKind::InvalidProfile,
                        "profile identity variant is invalid",
                    ));
                }
            }

            let key_store = object
                .get("key_store")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidProfile,
                        "profile key_store must be a JSON object",
                    )
                })?;
            if !key_store.get("namespaces").is_some_and(Value::is_object) {
                return Err(Error::new(
                    ErrorKind::InvalidProfile,
                    "profile key_store namespaces must be a JSON object",
                ));
            }

            controller_kind
        };

        Ok(Self {
            controller_kind,
            value,
        })
    }

    #[cfg(test)]
    pub(crate) const fn controller_kind(&self) -> ControllerKind {
        self.controller_kind
    }

    fn field_count(&self) -> usize {
        self.value.as_object().map_or(0, serde_json::Map::len)
    }
}

impl fmt::Debug for ProfileDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileDocument")
            .field("format", &PROFILE_FORMAT)
            .field("schema_version", &PROFILE_SCHEMA_VERSION)
            .field("controller_kind", &self.controller_kind)
            .field("identity", &Redacted)
            .field("key_store", &Redacted)
            .field("field_count", &self.field_count())
            .finish()
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T29 stores a validated pairing profile in ControllerConfig"
    )
)]
pub(crate) struct PairingProfile<M: ControllerModel> {
    document: ProfileDocument,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> PairingProfile<M> {
    #[cfg(test)]
    pub(crate) const fn controller_kind(&self) -> ControllerKind {
        M::KIND
    }
}

impl<M: ControllerModel> TryFrom<ProfileDocument> for PairingProfile<M> {
    type Error = Error;

    fn try_from(document: ProfileDocument) -> Result<Self, Self::Error> {
        if document.controller_kind != M::KIND {
            return Err(Error::new(
                ErrorKind::ProfileControllerMismatch,
                "profile controller kind does not match the requested model",
            ));
        }

        Ok(Self {
            document,
            _model: PhantomData,
        })
    }
}

impl<M: ControllerModel> fmt::Debug for PairingProfile<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingProfile")
            .field("controller_kind", &M::KIND)
            .field("document", &Redacted)
            .field("field_count", &self.document.field_count())
            .finish()
    }
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}
