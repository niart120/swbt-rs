use std::{collections::BTreeMap, fmt, marker::PhantomData};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{
    error::{Error, ErrorKind},
    model::ControllerModel,
};

use super::{ControllerKind, LocalAddress, ProfileIdentity, ProfileIdentityKind, ProfileSummary};

const PROFILE_SCHEMA_VERSION: u64 = 2;

/// Backend-independent Classic pairing material used by the runtime adapter.
#[doc(hidden)]
#[derive(Clone, PartialEq, Eq)]
pub struct StoredClassicBond {
    link_key: [u8; 16],
    link_key_type: u8,
    authenticated: bool,
}

impl StoredClassicBond {
    pub const fn new(link_key: [u8; 16], link_key_type: u8, authenticated: bool) -> Self {
        Self {
            link_key,
            link_key_type,
            authenticated,
        }
    }

    pub const fn link_key(&self) -> &[u8; 16] {
        &self.link_key
    }

    pub const fn link_key_type(&self) -> u8 {
        self.link_key_type
    }

    pub const fn authenticated(&self) -> bool {
        self.authenticated
    }
}

impl fmt::Debug for StoredClassicBond {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredClassicBond")
            .field("link_key", &"<redacted>")
            .field("link_key_type", &self.link_key_type)
            .field("authenticated", &self.authenticated)
            .finish()
    }
}

#[doc(hidden)]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDocument {
    controller_kind: DocumentControllerKind,
    format: DocumentFormat,
    identity: IdentityDocument,
    key_store: KeyStoreDocument,
    schema_version: u64,
}

impl ProfileDocument {
    pub fn empty_adapter_default<M: ControllerModel>() -> Self {
        Self::empty::<M>(ProfileIdentity::AdapterDefault)
    }

    pub fn empty<M: ControllerModel>(identity: ProfileIdentity) -> Self {
        Self {
            controller_kind: DocumentControllerKind::from(M::KIND),
            format: DocumentFormat::SwbtProfile,
            identity: IdentityDocument::from(identity),
            key_store: KeyStoreDocument::default(),
            schema_version: PROFILE_SCHEMA_VERSION,
        }
    }

    pub fn to_json_bytes(&self) -> crate::Result<Vec<u8>> {
        let value = serde_json::to_value(self).map_err(|source| {
            Error::with_source(
                ErrorKind::Internal,
                "profile document could not be serialized",
                source,
            )
        })?;
        let mut bytes = serde_json::to_vec_pretty(&value).map_err(|source| {
            Error::with_source(
                ErrorKind::Internal,
                "profile document could not be serialized",
                source,
            )
        })?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn parse_json(bytes: &[u8]) -> crate::Result<Self> {
        let document = serde_json::from_slice::<Self>(bytes).map_err(|source| {
            Error::with_source(
                ErrorKind::InvalidProfile,
                "profile document is not valid schema v2 JSON",
                source,
            )
        })?;
        document.validate()?;
        Ok(document)
    }

    fn validate(&self) -> crate::Result<()> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(invalid_profile("profile schema version must be 2"));
        }
        if self
            .key_store
            .namespaces
            .values()
            .any(|peers| peers.len() > 1)
        {
            return Err(invalid_profile(
                "profile key-store namespace must contain at most one current peer",
            ));
        }
        Ok(())
    }

    pub const fn controller_kind(&self) -> ControllerKind {
        self.controller_kind.as_public()
    }

    pub(super) fn summary(&self) -> ProfileSummary {
        ProfileSummary::new(
            PROFILE_SCHEMA_VERSION,
            self.controller_kind.as_public(),
            self.identity.kind(),
            self.key_store.namespaces.len(),
            self.key_store.namespaces.values().map(BTreeMap::len).sum(),
        )
    }
}

impl ProfileDocument {
    fn pairing_keys(&self, namespace: &str, peer: &str) -> Option<StoredClassicBond> {
        self.key_store
            .namespaces
            .get(namespace)
            .and_then(|peers| peers.get(peer))
            .map(ClassicPairingKeysDocument::to_stored_bond)
    }

    fn all_pairing_keys(&self, namespace: &str) -> Vec<(String, StoredClassicBond)> {
        self.key_store
            .namespaces
            .get(namespace)
            .into_iter()
            .flat_map(BTreeMap::iter)
            .map(|(peer, keys)| (peer.to_string(), keys.to_stored_bond()))
            .collect()
    }

    fn replace_pairing_keys(
        &mut self,
        namespace: &str,
        peer: &str,
        replacement: StoredClassicBond,
    ) -> crate::Result<()> {
        let namespace = BluetoothAddressKey::parse(namespace)?;
        let peer = ClassicPeerKey::parse(peer)?;
        let peers = self.key_store.namespaces.entry(namespace).or_default();
        peers.clear();
        peers.insert(
            peer,
            ClassicPairingKeysDocument::from_stored_bond(&replacement),
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
enum DocumentFormat {
    #[serde(rename = "swbt.profile")]
    SwbtProfile,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
enum DocumentControllerKind {
    #[serde(rename = "pro")]
    Pro,
    #[serde(rename = "joycon_l")]
    JoyConL,
    #[serde(rename = "joycon_r")]
    JoyConR,
}

impl DocumentControllerKind {
    const fn as_public(self) -> ControllerKind {
        match self {
            Self::Pro => ControllerKind::Pro,
            Self::JoyConL => ControllerKind::JoyConL,
            Self::JoyConR => ControllerKind::JoyConR,
        }
    }
}

impl From<ControllerKind> for DocumentControllerKind {
    fn from(value: ControllerKind) -> Self {
        match value {
            ControllerKind::Pro => Self::Pro,
            ControllerKind::JoyConL => Self::JoyConL,
            ControllerKind::JoyConR => Self::JoyConR,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind")]
enum IdentityDocument {
    #[serde(rename = "adapter-default")]
    AdapterDefault(AdapterDefaultIdentityDocument),
    #[serde(rename = "exp-local-address")]
    LocalAddress(LocalAddressIdentityDocument),
}

impl IdentityDocument {
    const fn as_public(&self) -> ProfileIdentity {
        match self {
            Self::AdapterDefault(_) => ProfileIdentity::AdapterDefault,
            Self::LocalAddress(identity) => ProfileIdentity::LocalAddress(identity.address.0),
        }
    }

    const fn kind(&self) -> ProfileIdentityKind {
        match self {
            Self::AdapterDefault(_) => ProfileIdentityKind::AdapterDefault,
            Self::LocalAddress(_) => ProfileIdentityKind::LocalAddress,
        }
    }
}

impl From<ProfileIdentity> for IdentityDocument {
    fn from(value: ProfileIdentity) -> Self {
        match value {
            ProfileIdentity::AdapterDefault => {
                Self::AdapterDefault(AdapterDefaultIdentityDocument {})
            }
            ProfileIdentity::LocalAddress(address) => {
                Self::LocalAddress(LocalAddressIdentityDocument {
                    address: LocalAddressDocument(address),
                })
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AdapterDefaultIdentityDocument {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LocalAddressIdentityDocument {
    address: LocalAddressDocument,
}

struct LocalAddressDocument(LocalAddress);

impl Serialize for LocalAddressDocument {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format_local_address(self.0))
    }
}

impl<'de> Deserialize<'de> for LocalAddressDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        LocalAddress::parse(&value)
            .map(Self)
            .map_err(|_| de::Error::custom("profile identity address is invalid"))
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeyStoreDocument {
    namespaces: BTreeMap<BluetoothAddressKey, BTreeMap<ClassicPeerKey, ClassicPairingKeysDocument>>,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct BluetoothAddressKey(String);

impl BluetoothAddressKey {
    fn parse(value: &str) -> crate::Result<Self> {
        if !is_canonical_bluetooth_address(value) {
            return Err(invalid_profile(
                "profile key-store namespace must use an uppercase Bluetooth address",
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for BluetoothAddressKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::borrow::Borrow<str> for BluetoothAddressKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Serialize for BluetoothAddressKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BluetoothAddressKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(|_| {
            de::Error::custom("profile key-store namespace must use an uppercase Bluetooth address")
        })
    }
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ClassicPeerKey(String);

impl ClassicPeerKey {
    fn parse(value: &str) -> crate::Result<Self> {
        let address = value.strip_suffix("/P").ok_or_else(|| {
            invalid_profile("profile key-store peer name must be a public Bluetooth address")
        })?;
        if !is_bluetooth_address(address) {
            return Err(invalid_profile(
                "profile key-store peer name must be a public Bluetooth address",
            ));
        }
        Ok(Self(format!("{}/P", address.to_ascii_uppercase())))
    }
}

impl fmt::Display for ClassicPeerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::borrow::Borrow<str> for ClassicPeerKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Serialize for ClassicPeerKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ClassicPeerKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(|_| {
            de::Error::custom("profile key-store peer name must be a public Bluetooth address")
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClassicPairingKeysDocument {
    link_key: LinkKeyDocument,
    link_key_type: u8,
}

impl ClassicPairingKeysDocument {
    fn to_stored_bond(&self) -> StoredClassicBond {
        StoredClassicBond::new(
            self.link_key.value.0,
            self.link_key_type,
            self.link_key.authenticated,
        )
    }

    fn from_stored_bond(value: &StoredClassicBond) -> Self {
        Self {
            link_key: LinkKeyDocument {
                authenticated: value.authenticated(),
                value: LinkKeyBytes(*value.link_key()),
            },
            link_key_type: value.link_key_type(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LinkKeyDocument {
    authenticated: bool,
    value: LinkKeyBytes,
}

struct LinkKeyBytes([u8; 16]);

impl Serialize for LinkKeyBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for LinkKeyBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        decode_link_key(&value)
            .map(Self)
            .map_err(|_| de::Error::custom("profile link_key value must be 16-byte hexadecimal"))
    }
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

fn is_canonical_bluetooth_address(value: &str) -> bool {
    is_bluetooth_address(value) && !value.bytes().any(|byte| byte.is_ascii_lowercase())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_link_key(value: &str) -> Result<[u8; 16], ()> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(());
    }
    let mut decoded = [0_u8; 16];
    for (output, pair) in decoded.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| ())?;
        *output = u8::from_str_radix(pair, 16).map_err(|_| ())?;
    }
    Ok(decoded)
}

fn invalid_profile(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidProfile, message)
}

impl fmt::Debug for ProfileDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileDocument")
            .field("format", &"swbt.profile")
            .field("schema_version", &self.schema_version)
            .field("controller_kind", &self.controller_kind.as_public())
            .field("identity", &Redacted)
            .field("key_store", &Redacted)
            .finish()
    }
}

/// A validated schema v2 pairing profile for one controller model.
///
/// The accepted document is the strict Classic pairing subset emitted by
/// swbt-python 0.6.0. Unknown fields and non-Classic key material are rejected.
/// Its [`Debug`](fmt::Debug) representation does not expose identity or
/// pairing-key values.
pub struct PairingProfile<M: ControllerModel> {
    document: ProfileDocument,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> PairingProfile<M> {
    #[doc(hidden)]
    pub fn empty(identity: ProfileIdentity) -> Self {
        Self {
            document: ProfileDocument::empty::<M>(identity),
            _model: PhantomData,
        }
    }

    /// Parses and validates a UTF-8 schema v2 profile document.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidProfile`] when the document is not valid
    /// JSON in the supported swbt-python 0.6.0 Classic profile shape.
    /// Returns [`ErrorKind::ProfileControllerMismatch`] when the document is
    /// for a controller model other than `M`.
    pub fn from_json(bytes: &[u8]) -> crate::Result<Self> {
        Self::try_from(ProfileDocument::parse_json(bytes)?)
    }

    /// Serializes the complete profile as deterministic UTF-8 JSON.
    ///
    /// Object keys are sorted, indentation is two spaces, Bluetooth addresses
    /// are uppercase, and the output ends with one newline.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Internal`] if the validated profile cannot be
    /// serialized.
    pub fn to_json_bytes(&self) -> crate::Result<Vec<u8>> {
        self.document.to_json_bytes()
    }

    /// Returns the controller model encoded by this typed profile.
    #[must_use]
    pub const fn controller_kind(&self) -> ControllerKind {
        M::KIND
    }

    #[doc(hidden)]
    pub const fn identity(&self) -> ProfileIdentity {
        self.document.identity.as_public()
    }
}

fn format_local_address(address: LocalAddress) -> String {
    let octets = address.octets();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        octets[0], octets[1], octets[2], octets[3], octets[4], octets[5]
    )
}

impl<M: ControllerModel> PairingProfile<M> {
    #[doc(hidden)]
    pub fn pairing_keys(&self, namespace: &str, peer: &str) -> Option<StoredClassicBond> {
        self.document.pairing_keys(namespace, peer)
    }

    #[doc(hidden)]
    pub fn all_pairing_keys(&self, namespace: &str) -> Vec<(String, StoredClassicBond)> {
        self.document.all_pairing_keys(namespace)
    }

    #[doc(hidden)]
    pub fn replace_pairing_keys(
        &mut self,
        namespace: &str,
        peer: &str,
        replacement: StoredClassicBond,
    ) -> crate::Result<()> {
        self.document
            .replace_pairing_keys(namespace, peer, replacement)
    }
}

impl<M: ControllerModel> TryFrom<ProfileDocument> for PairingProfile<M> {
    type Error = Error;

    fn try_from(document: ProfileDocument) -> Result<Self, Self::Error> {
        if document.controller_kind.as_public() != M::KIND {
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
            .finish()
    }
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}
