use std::{fmt, marker::PhantomData, path::PathBuf};

use serde_json::{Value, json};
use swbt_bumble_backend::{AddressKind, BluetoothAddress, BondStore, BondStoreError, ClassicBond};

use crate::{
    model::ControllerModel,
    profile::{FileProfileStore, PairingProfile, ProfileReadPort, ProfileUpdatePort},
};

pub(crate) struct ProfileKeyStoreFactory {
    create: Box<dyn Fn() -> ProfileKeyStore + Send>,
}

impl ProfileKeyStoreFactory {
    pub(crate) fn for_model<M: ControllerModel>(path: PathBuf) -> Self {
        Self {
            create: Box::new(move || {
                ProfileKeyStore(Box::new(SwbtProfileKeyStore::<M>::new(path.clone())))
            }),
        }
    }

    pub(super) fn create(&self) -> ProfileKeyStore {
        (self.create)()
    }
}

impl fmt::Debug for ProfileKeyStoreFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileKeyStoreFactory")
            .field("path", &Redacted)
            .finish_non_exhaustive()
    }
}

pub(super) struct ProfileKeyStore(Box<dyn BondStore>);

impl BondStore for ProfileKeyStore {
    fn select_local_address(
        &mut self,
        local_address: BluetoothAddress,
    ) -> Result<(), BondStoreError> {
        self.0.select_local_address(local_address)
    }

    fn load(&self, peer: BluetoothAddress) -> Result<Option<ClassicBond>, BondStoreError> {
        self.0.load(peer)
    }

    fn load_all(&self) -> Result<Vec<(BluetoothAddress, ClassicBond)>, BondStoreError> {
        self.0.load_all()
    }

    fn upsert(&mut self, peer: BluetoothAddress, bond: ClassicBond) -> Result<(), BondStoreError> {
        self.0.upsert(peer, bond)
    }
}

pub(super) struct SwbtProfileKeyStore<M: ControllerModel> {
    path: PathBuf,
    namespace: Option<String>,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> SwbtProfileKeyStore<M> {
    pub(super) fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            namespace: None,
            _model: PhantomData,
        }
    }

    fn namespace(&self, error: BondStoreError) -> Result<&str, BondStoreError> {
        self.namespace.as_deref().ok_or(error)
    }

    fn read_profile(
        &self,
        error: BondStoreError,
    ) -> Result<(Vec<u8>, PairingProfile<M>), BondStoreError> {
        let bytes = FileProfileStore.read(&self.path).map_err(|_| error)?;
        let profile = PairingProfile::from_json(&bytes).map_err(|_| error)?;
        Ok((bytes, profile))
    }

    fn commit(&self, expected: &[u8], profile: &PairingProfile<M>) -> Result<(), BondStoreError> {
        let replacement = profile
            .to_json_bytes()
            .map_err(|_| BondStoreError::UpsertFailed)?;
        FileProfileStore
            .update(&self.path, expected, &replacement)
            .map_err(|_| BondStoreError::UpsertFailed)
    }
}

impl<M: ControllerModel> BondStore for SwbtProfileKeyStore<M> {
    fn select_local_address(
        &mut self,
        local_address: BluetoothAddress,
    ) -> Result<(), BondStoreError> {
        self.namespace = Some(format_address(local_address));
        Ok(())
    }

    fn load(&self, peer: BluetoothAddress) -> Result<Option<ClassicBond>, BondStoreError> {
        let namespace = self.namespace(BondStoreError::LoadFailed)?;
        let (_, profile) = self.read_profile(BondStoreError::LoadFailed)?;
        profile
            .pairing_keys(namespace, &format_address(peer))
            .map(decode_bond)
            .transpose()
            .map_err(|_| BondStoreError::LoadFailed)
    }

    fn load_all(&self) -> Result<Vec<(BluetoothAddress, ClassicBond)>, BondStoreError> {
        let namespace = self.namespace(BondStoreError::ListFailed)?;
        let (_, profile) = self.read_profile(BondStoreError::ListFailed)?;
        profile
            .all_pairing_keys(namespace)
            .into_iter()
            .map(|(peer, value)| {
                let peer = BluetoothAddress::parse(&peer, AddressKind::Public)
                    .map_err(|_| BondStoreError::ListFailed)?;
                let bond = decode_bond(value).map_err(|_| BondStoreError::ListFailed)?;
                Ok((peer, bond))
            })
            .collect()
    }

    fn upsert(&mut self, peer: BluetoothAddress, bond: ClassicBond) -> Result<(), BondStoreError> {
        let namespace = self.namespace(BondStoreError::UpsertFailed)?.to_owned();
        let (expected, mut profile) = self.read_profile(BondStoreError::UpsertFailed)?;
        profile
            .replace_pairing_keys(&namespace, &format_address(peer), encode_bond(&bond))
            .map_err(|_| BondStoreError::UpsertFailed)?;
        self.commit(&expected, &profile)
    }
}

impl<M: ControllerModel> fmt::Debug for SwbtProfileKeyStore<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SwbtProfileKeyStore")
            .field("controller_kind", &M::KIND)
            .field("path", &Redacted)
            .field("namespace", &Redacted)
            .finish()
    }
}

fn encode_bond(bond: &ClassicBond) -> Value {
    json!({
        "address_type": 0,
        "link_key": {
            "authenticated": bond.authenticated(),
            "value": encode_hex(bond.link_key()),
        },
        "link_key_type": bond.link_key_type(),
    })
}

fn decode_bond(value: Value) -> Result<ClassicBond, ()> {
    let object = value.as_object().ok_or(())?;
    if object.get("address_type").and_then(Value::as_u64) != Some(0) {
        return Err(());
    }
    let link_key_type = object
        .get("link_key_type")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(())?;
    let link_key = object
        .get("link_key")
        .and_then(Value::as_object)
        .ok_or(())?;
    let authenticated = link_key
        .get("authenticated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let link_key = link_key.get("value").and_then(Value::as_str).ok_or(())?;
    let link_key = decode_link_key(link_key)?;
    Ok(ClassicBond::new(link_key, link_key_type, authenticated))
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

fn format_address(address: BluetoothAddress) -> String {
    let bytes = address.as_le_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[5], bytes[4], bytes[3], bytes[2], bytes[1], bytes[0]
    )
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use serde_json::{Value, json};
    use swbt_bumble_backend::{
        AddressKind, BluetoothAddress, BondStore, BondStoreError, ClassicBond,
    };

    use crate::model;

    use super::SwbtProfileKeyStore;

    const LOCAL_NAMESPACE: &str = "00:11:22:33:44:55";
    const ORIGINAL_PEER: &str = "98:B6:E9:11:22:33";
    const REPLACEMENT_PEER: &str = "98:B6:E9:44:55:66";
    const SECRET_SENTINEL: &str = "A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1";
    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn adapter_default_reads_only_the_resolved_local_namespace() {
        let temp = TempDirectory::new("read");
        let path = temp.path().join("pro.json");
        fs::write(&path, profile_bytes()).expect("write test profile");
        let mut store = SwbtProfileKeyStore::<model::Pro>::new(path.clone());

        assert_eq!(
            store.load(peer(ORIGINAL_PEER)),
            Err(BondStoreError::LoadFailed),
            "the backend must select the initialized adapter namespace first"
        );
        store
            .select_local_address(peer(LOCAL_NAMESPACE))
            .expect("select initialized adapter namespace");

        assert_eq!(
            store.load(peer(ORIGINAL_PEER)).expect("read current peer"),
            Some(bond(0xA1))
        );
        assert_eq!(
            store
                .load(peer("AA:BB:CC:DD:EE:FF"))
                .expect("ignore a peer from another namespace"),
            None
        );
        assert_eq!(
            store.load_all().expect("read current namespace"),
            [(peer(ORIGINAL_PEER), bond(0xA1))]
        );
        let rendered = format!("{store:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains(LOCAL_NAMESPACE));
        assert!(!rendered.contains(&path.to_string_lossy().into_owned()));
    }

    #[test]
    fn upsert_preserves_extensions_and_replaces_the_current_peer_atomically() {
        let temp = TempDirectory::new("update");
        let path = temp.path().join("pro.json");
        fs::write(&path, profile_bytes()).expect("write test profile");
        let mut store = selected_store(path.clone());

        store
            .upsert(peer(ORIGINAL_PEER), bond(0xB2))
            .expect("update the same peer");
        let same_peer: Value =
            serde_json::from_slice(&fs::read(&path).expect("read same-peer update"))
                .expect("updated profile remains JSON");
        assert_eq!(same_peer["future_top"], json!({"retained": true}));
        assert_eq!(
            same_peer["key_store"]["future_store"],
            json!({"retained": true})
        );
        assert_eq!(
            same_peer["key_store"]["namespaces"][LOCAL_NAMESPACE][ORIGINAL_PEER]["future_peer"],
            json!({"retained": true})
        );
        assert_eq!(
            store.load(peer(ORIGINAL_PEER)).expect("read updated peer"),
            Some(bond(0xB2))
        );

        store
            .upsert(peer(REPLACEMENT_PEER), bond(0xC3))
            .expect("replace the current peer");
        assert_eq!(
            store.load_all().expect("read replacement peer"),
            [(peer(REPLACEMENT_PEER), bond(0xC3))]
        );
        assert_eq!(
            store
                .load(peer(ORIGINAL_PEER))
                .expect("old peer was removed"),
            None
        );
    }

    #[test]
    fn invalid_profile_failures_are_typed_and_secret_free() {
        let temp = TempDirectory::new("secret-path");
        let path = temp.path().join("secret-profile-name.json");
        fs::write(&path, profile_bytes()).expect("write test profile");
        let mut store = selected_store(path.clone());

        fs::write(&path, b"{\"broken\":\"profile\"}").expect("corrupt test profile");
        let load = store
            .load(peer(ORIGINAL_PEER))
            .expect_err("invalid profile must fail key lookup");
        assert_eq!(load, BondStoreError::LoadFailed);
        assert_secret_free(&load, &path);

        let list = store
            .load_all()
            .expect_err("invalid profile must fail bond listing");
        assert_eq!(list, BondStoreError::ListFailed);
        assert_secret_free(&list, &path);

        let upsert = store
            .upsert(peer(ORIGINAL_PEER), bond(0xD4))
            .expect_err("invalid profile must fail bond update");
        assert_eq!(upsert, BondStoreError::UpsertFailed);
        assert_secret_free(&upsert, &path);
    }

    fn selected_store(path: PathBuf) -> SwbtProfileKeyStore<model::Pro> {
        let mut store = SwbtProfileKeyStore::new(path);
        store
            .select_local_address(peer(LOCAL_NAMESPACE))
            .expect("select initialized adapter namespace");
        store
    }

    fn peer(value: &str) -> BluetoothAddress {
        BluetoothAddress::parse(value, AddressKind::Public).expect("valid test address")
    }

    fn bond(byte: u8) -> ClassicBond {
        ClassicBond::new([byte; 16], 4, true)
    }

    fn profile_bytes() -> Vec<u8> {
        serde_json::to_vec_pretty(&json!({
            "format": "swbt.profile",
            "schema_version": 2,
            "controller_kind": "pro",
            "identity": {
                "kind": "adapter-default"
            },
            "key_store": {
                "namespaces": {
                    LOCAL_NAMESPACE: {
                        ORIGINAL_PEER: {
                            "address_type": 0,
                            "link_key": {
                                "authenticated": true,
                                "value": SECRET_SENTINEL
                            },
                            "link_key_type": 4,
                            "future_peer": {
                                "retained": true
                            }
                        }
                    },
                    "10:11:22:33:44:55": {
                        "AA:BB:CC:DD:EE:FF": {
                            "address_type": 0,
                            "link_key": {
                                "authenticated": true,
                                "value": "D4D4D4D4D4D4D4D4D4D4D4D4D4D4D4D4"
                            },
                            "link_key_type": 4
                        }
                    }
                },
                "future_store": {
                    "retained": true
                }
            },
            "future_top": {
                "retained": true
            }
        }))
        .expect("serialize test profile")
    }

    fn assert_secret_free(error: &BondStoreError, path: &Path) {
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(!rendered.contains(SECRET_SENTINEL));
            assert!(!rendered.contains(ORIGINAL_PEER));
            assert!(!rendered.contains(&path.to_string_lossy().into_owned()));
            assert!(!rendered.contains("secret-profile-name"));
        }
    }

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "swbt-rs-profile-key-store-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create unique test directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).expect("remove test directory");
        }
    }
}
