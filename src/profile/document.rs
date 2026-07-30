use std::{fmt, marker::PhantomData};

use serde_json::Value;

use crate::{
    error::{Error, ErrorKind},
    model::ControllerModel,
};

use super::{ControllerKind, LocalAddress};

const PROFILE_FORMAT: &str = "swbt.profile";
const PROFILE_SCHEMA_VERSION: u64 = 2;

pub(crate) struct ProfileDocument {
    controller_kind: ControllerKind,
    value: Value,
}

impl ProfileDocument {
    pub(crate) fn empty_adapter_default<M: ControllerModel>() -> Self {
        Self {
            controller_kind: M::KIND,
            value: serde_json::json!({
                "format": PROFILE_FORMAT,
                "schema_version": PROFILE_SCHEMA_VERSION,
                "controller_kind": M::KIND.profile_name(),
                "identity": {
                    "kind": "adapter-default"
                },
                "key_store": {
                    "namespaces": {}
                }
            }),
        }
    }

    pub(crate) fn to_json_bytes(&self) -> crate::Result<Vec<u8>> {
        serde_json::to_vec(&self.value).map_err(|source| {
            Error::with_source(
                ErrorKind::Internal,
                "profile document could not be serialized",
                source,
            )
        })
    }

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
                Some("exp-local-address") => {
                    let address =
                        identity
                            .get("address")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::InvalidProfile,
                                    "profile identity address must be a string",
                                )
                            })?;
                    LocalAddress::parse(address).map_err(|source| {
                        Error::with_source(
                            ErrorKind::InvalidProfile,
                            "profile identity address is invalid",
                            source,
                        )
                    })?;
                }
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
            let namespaces = key_store
                .get("namespaces")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    invalid_profile("profile key_store namespaces must be a JSON object")
                })?;
            validate_namespaces(namespaces)?;

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

fn validate_namespaces(namespaces: &serde_json::Map<String, Value>) -> crate::Result<()> {
    for (namespace, peers) in namespaces {
        if !is_bluetooth_address(namespace) {
            return Err(invalid_profile(
                "profile key-store namespace must be a Bluetooth address",
            ));
        }
        let peers = peers.as_object().ok_or_else(|| {
            invalid_profile("profile key-store namespace value must be a JSON object")
        })?;
        if peers.len() > 1 {
            return Err(invalid_profile(
                "profile key-store namespace must contain at most one current peer",
            ));
        }
        for (peer, keys) in peers {
            if !is_bluetooth_address(peer) {
                return Err(invalid_profile(
                    "profile key-store peer name must be a Bluetooth address",
                ));
            }
            validate_pairing_keys(keys)?;
        }
    }
    Ok(())
}

fn validate_pairing_keys(keys: &Value) -> crate::Result<()> {
    let keys = keys
        .as_object()
        .ok_or_else(|| invalid_profile("profile pairing keys must be a JSON object"))?;

    for field in ["address_type", "link_key_type"] {
        if let Some(value) = keys.get(field) {
            if !is_unsigned_integer_at_most(value, u64::from(u8::MAX)) {
                return Err(invalid_profile(
                    "profile pairing key numeric field must be an unsigned byte",
                ));
            }
        }
    }
    for field in [
        "ltk",
        "ltk_central",
        "ltk_peripheral",
        "irk",
        "csrk",
        "local_csrk",
        "link_key",
    ] {
        if let Some(value) = keys.get(field) {
            validate_key(value)?;
        }
    }
    Ok(())
}

fn validate_key(key: &Value) -> crate::Result<()> {
    let key = key
        .as_object()
        .ok_or_else(|| invalid_profile("profile key field must be a JSON object"))?;
    let value = key
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_profile("profile key value must be a hexadecimal string"))?;
    if !is_even_hex(value) {
        return Err(invalid_profile(
            "profile key value must be an even-length hexadecimal string",
        ));
    }
    if key
        .get("authenticated")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(invalid_profile(
            "profile key authenticated field must be a boolean",
        ));
    }
    if let Some(value) = key.get("ediv") {
        if !is_unsigned_integer_at_most(value, u64::from(u16::MAX)) {
            return Err(invalid_profile(
                "profile key ediv field must be an unsigned 16-bit integer",
            ));
        }
    }
    if let Some(value) = key.get("rand") {
        let value = value
            .as_str()
            .ok_or_else(|| invalid_profile("profile key rand field must be hexadecimal"))?;
        if !is_even_hex(value) {
            return Err(invalid_profile(
                "profile key rand field must be even-length hexadecimal",
            ));
        }
    }
    if let Some(value) = key.get("sign_counter") {
        if !is_unsigned_integer_at_most(value, u64::from(u32::MAX)) {
            return Err(invalid_profile(
                "profile key sign_counter field must be an unsigned 32-bit integer",
            ));
        }
    }
    Ok(())
}

fn is_bluetooth_address(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 17
        && [2, 5, 8, 11, 14]
            .into_iter()
            .all(|index| bytes[index] == b':')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 2 | 5 | 8 | 11 | 14) || byte.is_ascii_hexdigit())
}

fn is_even_hex(value: &str) -> bool {
    !value.is_empty()
        && value.len().is_multiple_of(2)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_unsigned_integer_at_most(value: &Value, maximum: u64) -> bool {
    value.as_u64().is_some_and(|value| value <= maximum)
}

fn invalid_profile(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidProfile, message)
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
