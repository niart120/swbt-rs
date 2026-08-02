use std::{fmt, marker::PhantomData, path::PathBuf};

use swbt_bumble_backend::{AddressKind, BluetoothAddress, BondStore, BondStoreError, ClassicBond};

use crate::{
    model::ControllerModel,
    profile::{FileProfileStore, PairingProfile, ProfileStore},
};

pub(crate) struct ProfileKeyStoreFactory {
    create: Box<dyn Fn() -> Box<dyn BondStore> + Send>,
}

impl ProfileKeyStoreFactory {
    pub(crate) fn for_model<M: ControllerModel>(path: PathBuf) -> Self {
        Self {
            create: Box::new(move || Box::new(SwbtProfileKeyStore::<M>::new(path.clone()))),
        }
    }

    pub(super) fn create(&self) -> Box<dyn BondStore> {
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

    fn read_profile(&self, error: BondStoreError) -> Result<PairingProfile<M>, BondStoreError> {
        let bytes = FileProfileStore.read(&self.path).map_err(|_| error)?;
        PairingProfile::from_json(&bytes).map_err(|_| error)
    }

    fn commit(&self, profile: &PairingProfile<M>) -> Result<(), BondStoreError> {
        let replacement = profile
            .to_json_bytes()
            .map_err(|_| BondStoreError::UpsertFailed)?;
        FileProfileStore
            .update(&self.path, &replacement)
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
        let profile = self.read_profile(BondStoreError::LoadFailed)?;
        let public_peer = format_public_peer(peer);
        Ok(profile.pairing_keys(namespace, &public_peer))
    }

    fn load_all(&self) -> Result<Vec<(BluetoothAddress, ClassicBond)>, BondStoreError> {
        let namespace = self.namespace(BondStoreError::ListFailed)?;
        let profile = self.read_profile(BondStoreError::ListFailed)?;
        profile
            .all_pairing_keys(namespace)
            .into_iter()
            .map(|(peer, value)| {
                let raw_peer = peer.strip_suffix("/P").ok_or(BondStoreError::ListFailed)?;
                let peer = BluetoothAddress::parse(raw_peer, AddressKind::Public)
                    .map_err(|_| BondStoreError::ListFailed)?;
                Ok((peer, value))
            })
            .collect()
    }

    fn upsert(&mut self, peer: BluetoothAddress, bond: ClassicBond) -> Result<(), BondStoreError> {
        let namespace = self.namespace(BondStoreError::UpsertFailed)?.to_owned();
        let mut profile = self.read_profile(BondStoreError::UpsertFailed)?;
        profile
            .replace_pairing_keys(&namespace, &format_public_peer(peer), bond)
            .map_err(|_| BondStoreError::UpsertFailed)?;
        self.commit(&profile)
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

fn format_address(address: BluetoothAddress) -> String {
    let bytes = address.as_le_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[5], bytes[4], bytes[3], bytes[2], bytes[1], bytes[0]
    )
}

fn format_public_peer(address: BluetoothAddress) -> String {
    format!("{}/P", format_address(address))
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
    const ORIGINAL_PUBLIC_PEER: &str = "98:B6:E9:11:22:33/P";
    const REPLACEMENT_PEER: &str = "98:B6:E9:44:55:66";
    const REPLACEMENT_PUBLIC_PEER: &str = "98:B6:E9:44:55:66/P";
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
    fn upsert_writes_the_python_classic_shape_and_replaces_the_current_peer_atomically() {
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
        assert!(
            same_peer["key_store"]["namespaces"][LOCAL_NAMESPACE][ORIGINAL_PUBLIC_PEER]
                .get("address_type")
                .is_none(),
            "Rust writes the same classic bond shape as Python 0.6.0"
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
        let replacement: Value =
            serde_json::from_slice(&fs::read(&path).expect("read replacement update"))
                .expect("replacement profile remains JSON");
        assert!(
            replacement["key_store"]["namespaces"][LOCAL_NAMESPACE]
                .get(REPLACEMENT_PUBLIC_PEER)
                .is_some()
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
                        ORIGINAL_PUBLIC_PEER: {
                            "link_key": {
                                "authenticated": true,
                                "value": SECRET_SENTINEL
                            },
                            "link_key_type": 4,
                        }
                    },
                    "10:11:22:33:44:55": {
                        "AA:BB:CC:DD:EE:FF/P": {
                            "link_key": {
                                "authenticated": true,
                                "value": "D4D4D4D4D4D4D4D4D4D4D4D4D4D4D4D4"
                            },
                            "link_key_type": 4
                        }
                    }
                },
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
