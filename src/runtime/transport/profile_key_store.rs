#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M6 T05 attaches the T04 key-store adapter to the Bumble device"
    )
)]

use std::{fmt, io, marker::PhantomData, path::PathBuf};

use bumble::keys::{KeyStore, KeyStoreError, KeyStoreResult, PairingKeys};
use serde_json::Value;

use crate::{
    model::ControllerModel,
    profile::{FileProfileStore, PairingProfile, ProfileReadPort, ProfileUpdatePort},
};

pub(super) struct SwbtProfileKeyStore<M: ControllerModel> {
    path: PathBuf,
    namespace: String,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> SwbtProfileKeyStore<M> {
    pub(super) fn new(path: impl Into<PathBuf>, local_address: [u8; 6]) -> Self {
        Self {
            path: path.into(),
            namespace: format_local_address(local_address),
            _model: PhantomData,
        }
    }

    fn read_profile(&self) -> KeyStoreResult<(Vec<u8>, PairingProfile<M>)> {
        let bytes = FileProfileStore
            .read(&self.path)
            .map_err(|source| sanitized_io(source.kind(), "pairing profile could not be read"))?;
        let profile = PairingProfile::from_json(&bytes)
            .map_err(|_| sanitized_invalid_data("pairing profile is invalid"))?;
        Ok((bytes, profile))
    }

    fn commit(&self, expected: &[u8], profile: &PairingProfile<M>) -> KeyStoreResult<()> {
        let replacement = profile
            .to_json_bytes()
            .map_err(|_| sanitized_invalid_data("pairing profile could not be serialized"))?;
        FileProfileStore
            .update(&self.path, expected, &replacement)
            .map_err(|source| sanitized_io(source.kind(), "pairing profile could not be updated"))
    }
}

impl<M: ControllerModel> KeyStore for SwbtProfileKeyStore<M> {
    fn delete(&mut self, name: &str) -> KeyStoreResult<()> {
        let (expected, mut profile) = self.read_profile()?;
        let removed = profile
            .remove_pairing_keys(&self.namespace, name)
            .map_err(|_| sanitized_invalid_address())?;
        if !removed {
            return Err(KeyStoreError::NotFound("<redacted>".to_owned()));
        }
        self.commit(&expected, &profile)
    }

    fn update(&mut self, name: &str, keys: PairingKeys) -> KeyStoreResult<()> {
        let replacement = encode_pairing_keys(&keys)?;
        let (expected, mut profile) = self.read_profile()?;
        profile
            .replace_pairing_keys(&self.namespace, name, replacement)
            .map_err(|_| sanitized_invalid_address())?;
        self.commit(&expected, &profile)
    }

    fn get(&self, name: &str) -> KeyStoreResult<Option<PairingKeys>> {
        let (_, profile) = self.read_profile()?;
        profile
            .pairing_keys(&self.namespace, name)
            .map(decode_pairing_keys)
            .transpose()
    }

    fn get_all(&self) -> KeyStoreResult<Vec<(String, PairingKeys)>> {
        let (_, profile) = self.read_profile()?;
        profile
            .all_pairing_keys(&self.namespace)
            .into_iter()
            .map(|(peer, value)| decode_pairing_keys(value).map(|keys| (peer, keys)))
            .collect()
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

fn encode_pairing_keys(keys: &PairingKeys) -> KeyStoreResult<Value> {
    let json = keys
        .to_json()
        .map_err(|_| sanitized_invalid_data("pairing keys could not be serialized"))?;
    serde_json::from_str(&json)
        .map_err(|_| sanitized_invalid_data("pairing keys could not be serialized"))
}

fn decode_pairing_keys(value: Value) -> KeyStoreResult<PairingKeys> {
    let json = serde_json::to_string(&value)
        .map_err(|_| sanitized_invalid_data("stored pairing keys are invalid"))?;
    PairingKeys::from_json(&json)
        .map_err(|_| sanitized_invalid_data("stored pairing keys are invalid"))
}

fn format_local_address(address: [u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        address[0], address[1], address[2], address[3], address[4], address[5]
    )
}

fn sanitized_invalid_address() -> KeyStoreError {
    KeyStoreError::InvalidAddress("<redacted>".to_owned())
}

fn sanitized_invalid_data(message: &'static str) -> KeyStoreError {
    sanitized_io(io::ErrorKind::InvalidData, message)
}

fn sanitized_io(kind: io::ErrorKind, message: &'static str) -> KeyStoreError {
    KeyStoreError::Io(io::Error::new(kind, message))
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

    use bumble::{
        AddressType,
        keys::{Key, KeyStore, KeyStoreError, PairingKeys},
    };
    use serde_json::{Value, json};

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
        let store = SwbtProfileKeyStore::<model::Pro>::new(
            path.clone(),
            [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
        );

        assert_eq!(
            store.get(ORIGINAL_PEER).expect("read current peer"),
            Some(pairing_keys(0xA1))
        );
        assert_eq!(
            store
                .get("AA:BB:CC:DD:EE:FF")
                .expect("ignore a peer from another namespace"),
            None
        );
        assert_eq!(
            store.get_all().expect("read current namespace"),
            [(ORIGINAL_PEER.to_owned(), pairing_keys(0xA1))]
        );
        let rendered = format!("{store:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains(LOCAL_NAMESPACE));
        assert!(!rendered.contains(&path.to_string_lossy().into_owned()));
    }

    #[test]
    fn update_preserves_extensions_and_replaces_the_current_peer_atomically() {
        let temp = TempDirectory::new("update");
        let path = temp.path().join("pro.json");
        fs::write(&path, profile_bytes()).expect("write test profile");
        let mut store =
            SwbtProfileKeyStore::<model::Pro>::new(path.clone(), [0, 0x11, 0x22, 0x33, 0x44, 0x55]);

        store
            .update(ORIGINAL_PEER, pairing_keys(0xB2))
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
            store.get(ORIGINAL_PEER).expect("read updated peer"),
            Some(pairing_keys(0xB2))
        );

        store
            .update(REPLACEMENT_PEER, pairing_keys(0xC3))
            .expect("replace the current peer");
        assert_eq!(
            store.get_all().expect("read replacement peer"),
            [(REPLACEMENT_PEER.to_owned(), pairing_keys(0xC3))]
        );
        assert_eq!(
            store.get(ORIGINAL_PEER).expect("old peer was removed"),
            None
        );
    }

    #[test]
    fn delete_is_explicit_and_failures_do_not_expose_names_paths_or_keys() {
        let temp = TempDirectory::new("delete-secret-path");
        let path = temp.path().join("secret-profile-name.json");
        fs::write(&path, profile_bytes()).expect("write test profile");
        let mut store =
            SwbtProfileKeyStore::<model::Pro>::new(path.clone(), [0, 0x11, 0x22, 0x33, 0x44, 0x55]);

        store
            .delete(ORIGINAL_PEER)
            .expect("explicitly delete the current peer");
        assert!(store.get_all().expect("read empty namespace").is_empty());
        let after_delete: Value =
            serde_json::from_slice(&fs::read(&path).expect("read deletion update"))
                .expect("profile remains JSON after deletion");
        assert_eq!(after_delete["future_top"], json!({"retained": true}));
        assert_eq!(
            after_delete["key_store"]["future_store"],
            json!({"retained": true})
        );

        let missing = store
            .delete("FF:EE:DD:CC:BB:AA")
            .expect_err("missing peer must not be treated as deleted");
        assert!(matches!(missing, KeyStoreError::NotFound(_)));
        assert_secret_free(&missing, &path);

        let invalid_name = store
            .delete(SECRET_SENTINEL)
            .expect_err("invalid peer name must fail deletion");
        assert!(matches!(invalid_name, KeyStoreError::InvalidAddress(_)));
        assert_secret_free(&invalid_name, &path);

        fs::write(&path, b"{\"broken\":\"profile\"}").expect("corrupt test profile");
        let invalid = store
            .get(ORIGINAL_PEER)
            .expect_err("invalid profile must fail key lookup");
        assert_secret_free(&invalid, &path);
    }

    fn pairing_keys(byte: u8) -> PairingKeys {
        PairingKeys {
            address_type: Some(AddressType::PUBLIC_DEVICE),
            link_key: Some(Key {
                value: vec![byte; 16],
                authenticated: true,
                ..Key::default()
            }),
            link_key_type: Some(4),
            ..PairingKeys::default()
        }
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
                            "link_key": {
                                "value": "D4D4D4D4D4D4D4D4D4D4D4D4D4D4D4D4"
                            }
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

    fn assert_secret_free(error: &KeyStoreError, path: &Path) {
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
