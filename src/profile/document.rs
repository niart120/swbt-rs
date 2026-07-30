use std::{fmt, marker::PhantomData};

use serde_json::{Map, Value};

use crate::{
    error::{Error, ErrorKind},
    model::ControllerModel,
};

use super::{ControllerKind, LocalAddress};

const PROFILE_FORMAT: &str = "swbt.profile";
const PROFILE_SCHEMA_VERSION: u64 = 2;
#[allow(
    dead_code,
    reason = "M6 T05 attaches the T04 key-store adapter to the Bumble device"
)]
const KNOWN_PAIRING_KEY_FIELDS: [&str; 9] = [
    "address_type",
    "ltk",
    "ltk_central",
    "ltk_peripheral",
    "irk",
    "csrk",
    "local_csrk",
    "link_key",
    "link_key_type",
];

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
        let mut bytes = serde_json::to_vec_pretty(&self.value).map_err(|source| {
            Error::with_source(
                ErrorKind::Internal,
                "profile document could not be serialized",
                source,
            )
        })?;
        bytes.push(b'\n');
        Ok(bytes)
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

#[allow(
    dead_code,
    reason = "M6 T05 attaches the T04 key-store adapter to the Bumble device"
)]
impl ProfileDocument {
    fn pairing_keys(&self, namespace: &str, peer: &str) -> Option<Value> {
        self.namespaces()
            .and_then(|namespaces| namespaces.get(namespace))
            .and_then(Value::as_object)
            .and_then(|peers| peers.get(peer))
            .cloned()
    }

    fn all_pairing_keys(&self, namespace: &str) -> Vec<(String, Value)> {
        self.namespaces()
            .and_then(|namespaces| namespaces.get(namespace))
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|peers| peers.iter())
            .map(|(peer, keys)| (peer.clone(), keys.clone()))
            .collect()
    }

    fn replace_pairing_keys(
        &mut self,
        namespace: &str,
        peer: &str,
        replacement: Value,
    ) -> crate::Result<()> {
        if !is_bluetooth_address(namespace) || !is_bluetooth_address(peer) {
            return Err(invalid_profile(
                "profile key-store update address is invalid",
            ));
        }
        validate_pairing_keys(&replacement)?;
        let mut replacement = replacement.as_object().cloned().ok_or_else(|| {
            invalid_profile("profile key-store update must contain a pairing-key object")
        })?;
        let peers = self.namespace_mut(namespace)?;

        if let Some(existing) = peers.get_mut(peer).and_then(Value::as_object_mut) {
            for field in KNOWN_PAIRING_KEY_FIELDS {
                existing.remove(field);
            }
            existing.append(&mut replacement);
        } else {
            peers.clear();
            peers.insert(peer.to_owned(), Value::Object(replacement));
        }
        Ok(())
    }

    fn remove_pairing_keys(&mut self, namespace: &str, peer: &str) -> crate::Result<bool> {
        if !is_bluetooth_address(namespace) || !is_bluetooth_address(peer) {
            return Err(invalid_profile(
                "profile key-store delete address is invalid",
            ));
        }
        let Some(namespaces) = self.namespaces_mut()? else {
            return Ok(false);
        };
        let Some(peers) = namespaces.get_mut(namespace).and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        Ok(peers.remove(peer).is_some())
    }

    fn namespaces(&self) -> Option<&Map<String, Value>> {
        self.value
            .as_object()
            .and_then(|document| document.get("key_store"))
            .and_then(Value::as_object)
            .and_then(|key_store| key_store.get("namespaces"))
            .and_then(Value::as_object)
    }

    fn namespaces_mut(&mut self) -> crate::Result<Option<&mut Map<String, Value>>> {
        let namespaces = self
            .value
            .as_object_mut()
            .and_then(|document| document.get_mut("key_store"))
            .and_then(Value::as_object_mut)
            .and_then(|key_store| key_store.get_mut("namespaces"));
        match namespaces {
            Some(Value::Object(namespaces)) => Ok(Some(namespaces)),
            Some(_) => Err(internal_profile_shape_error()),
            None => Ok(None),
        }
    }

    fn namespace_mut(&mut self, namespace: &str) -> crate::Result<&mut Map<String, Value>> {
        let namespaces = self
            .namespaces_mut()?
            .ok_or_else(internal_profile_shape_error)?;
        let peers = namespaces
            .entry(namespace.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        peers
            .as_object_mut()
            .ok_or_else(internal_profile_shape_error)
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

#[allow(
    dead_code,
    reason = "M6 T05 attaches the T04 key-store adapter to the Bumble device"
)]
fn internal_profile_shape_error() -> Error {
    Error::new(
        ErrorKind::Internal,
        "validated profile document shape changed unexpectedly",
    )
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

/// A validated schema v2 pairing profile for one controller model.
///
/// The type retains unknown JSON fields so that reading and writing a profile
/// does not discard extensions created by another compatible implementation.
/// Its [`Debug`](fmt::Debug) representation does not expose the raw document or
/// pairing-key values.
pub struct PairingProfile<M: ControllerModel> {
    document: ProfileDocument,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> PairingProfile<M> {
    /// Parses and validates a UTF-8 schema v2 profile document.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidProfile`] when the document is not valid
    /// schema v2 JSON or contains an invalid identity or key-store shape.
    /// Returns [`ErrorKind::ProfileControllerMismatch`] when the document is
    /// for a controller model other than `M`.
    pub fn from_json(bytes: &[u8]) -> crate::Result<Self> {
        Self::try_from(ProfileDocument::parse_json(bytes)?)
    }

    /// Serializes the complete profile as deterministic UTF-8 JSON.
    ///
    /// Object keys are sorted, indentation is two spaces, and the output ends
    /// with one newline. Unknown fields retained during parsing are included.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Internal`] if the retained JSON document cannot be
    /// serialized.
    pub fn to_json_bytes(&self) -> crate::Result<Vec<u8>> {
        self.document.to_json_bytes()
    }

    /// Returns the controller model encoded by this typed profile.
    #[must_use]
    pub const fn controller_kind(&self) -> ControllerKind {
        M::KIND
    }
}

#[allow(
    dead_code,
    reason = "M6 T05 attaches the T04 key-store adapter to the Bumble device"
)]
impl<M: ControllerModel> PairingProfile<M> {
    pub(crate) fn pairing_keys(&self, namespace: &str, peer: &str) -> Option<Value> {
        self.document.pairing_keys(namespace, peer)
    }

    pub(crate) fn all_pairing_keys(&self, namespace: &str) -> Vec<(String, Value)> {
        self.document.all_pairing_keys(namespace)
    }

    pub(crate) fn replace_pairing_keys(
        &mut self,
        namespace: &str,
        peer: &str,
        replacement: Value,
    ) -> crate::Result<()> {
        self.document
            .replace_pairing_keys(namespace, peer, replacement)
    }

    pub(crate) fn remove_pairing_keys(
        &mut self,
        namespace: &str,
        peer: &str,
    ) -> crate::Result<bool> {
        self.document.remove_pairing_keys(namespace, peer)
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
