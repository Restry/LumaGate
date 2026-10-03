use super::*;
use crate::manual::keys::FileKeyStore;
use std::{cell::RefCell, collections::HashMap};
const A: &str = "fixture-key-a-0123456789abcdef0123456789abcdef";
const B: &str = "fixture-key-b-0123456789abcdef0123456789abcdef";
#[derive(Default)]
struct Store {
    values: RefCell<HashMap<String, String>>,
    fail_set: bool,
    fail_delete: bool,
}
impl KeyStore for Store {
    fn get(&self, id: &str) -> Result<String, String> {
        self.values
            .borrow()
            .get(id)
            .cloned()
            .ok_or("missing".into())
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        if self.fail_set {
            return Err("write failed".into());
        }
        self.values.borrow_mut().insert(id.into(), value.into());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        if self.fail_delete {
            return Err("delete failed".into());
        }
        self.values.borrow_mut().remove(id);
        Ok(())
    }
}
fn apply(db: &Database, store: &impl KeyStore, operation: Operation) -> ChangeResult {
    let current = Registry::load(db).unwrap();
    current
        .clone()
        .change(current.revision, operation, store, |value| {
            db.set_setting(REGISTRY_KEY, value)
                .map_err(|e| e.to_string())
        })
        .unwrap()
}
fn add(db: &Database, store: &impl KeyStore, name: &str, value: &str) -> String {
    let result = apply(
        db,
        store,
        Operation::Add {
            name: name.into(),
            value: value.into(),
        },
    );
    result
        .keys
        .items
        .iter()
        .find(|item| item.name == name)
        .unwrap()
        .id
        .clone()
}
#[test]
fn multiple_keys_can_be_copied_after_reload_without_leaking_in_views_or_sqlite() {
    let db = Database::memory().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = FileKeyStore::at_directory(directory.path().join("access-keys"));
    let a = add(&db, &store, "MacBook", A);
    let b = add(&db, &store, "CI", B);
    let loaded = Registry::load(&db).unwrap();
    assert_eq!(loaded.reveal(&a, &store).unwrap(), A);
    assert_eq!(
        loaded
            .reveal(
                &b,
                &FileKeyStore::at_directory(directory.path().join("access-keys"))
            )
            .unwrap(),
        B
    );
    assert_eq!(loaded.identify(A).unwrap().name, "MacBook");
    for public in [
        serde_json::to_string(&loaded.view()).unwrap(),
        db.get_setting(REGISTRY_KEY).unwrap().unwrap(),
    ] {
        assert!(!public.contains(A));
        assert!(!public.contains(B));
    }
    let view = serde_json::to_string(&loaded.view()).unwrap();
    assert!(!view.contains("secretRef"));
    assert!(!view.contains("tag"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = directory.path().join("access-keys");
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for path in std::fs::read_dir(dir).unwrap() {
            assert_eq!(
                path.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
#[test]
fn legacy_tag_is_preserved_without_writes_and_only_matching_plaintext_can_enable_copy() {
    let db = Database::memory().unwrap();
    let store = Store::default();
    let mut legacy = Settings::default();
    legacy.access_key_tag = Some(
        super::super::credential_mac(A)
            .finalize()
            .into_bytes()
            .to_vec(),
    );
    db.set_setting(
        super::super::SETTINGS_KEY,
        &serde_json::to_string(&legacy).unwrap(),
    )
    .unwrap();
    let registry = Registry::load(&db).unwrap();
    assert!(db.get_setting(REGISTRY_KEY).unwrap().is_none());
    assert!(registry.identify(A).is_some());
    assert!(!registry.view().items[0].can_copy);
    assert!(registry.reveal("legacy", &store).is_err());
    assert!(registry
        .change(
            0,
            Operation::RestoreCopy {
                key_id: "legacy".into(),
                value: B.into()
            },
            &store,
            |_| panic!("must not commit mismatch")
        )
        .is_err());
    assert!(store.values.borrow().is_empty());
    apply(
        &db,
        &store,
        Operation::RestoreCopy {
            key_id: "legacy".into(),
            value: A.into(),
        },
    );
    let updated = Registry::load(&db).unwrap();
    assert_eq!(updated.reveal("legacy", &store).unwrap(), A);
    assert_eq!(updated.identify(A).unwrap().id, "legacy");
    apply(
        &db,
        &store,
        Operation::Remove {
            key_id: "legacy".into(),
        },
    );
    assert!(Registry::load(&db).unwrap().view().items.is_empty());
    assert!(Registry::load(&db).unwrap().identify(A).is_none());
    assert!(store.values.borrow().is_empty());
}
#[test]
fn rename_disable_remove_preserve_identity_and_leave_unrelated_keys_usable() {
    let db = Database::memory().unwrap();
    let store = Store::default();
    let a = add(&db, &store, "A", A);
    let b = add(&db, &store, "B", B);
    let before = Registry::load(&db).unwrap().identify(A).unwrap();
    apply(
        &db,
        &store,
        Operation::Rename {
            key_id: a.clone(),
            name: "A renamed".into(),
        },
    );
    let renamed = Registry::load(&db).unwrap().identify(A).unwrap();
    assert_eq!(renamed.id, before.id);
    assert_eq!(before.name, "A");
    apply(
        &db,
        &store,
        Operation::SetEnabled {
            key_id: a.clone(),
            enabled: false,
        },
    );
    let disabled = Registry::load(&db).unwrap();
    assert!(disabled.identify(A).is_none());
    assert!(disabled.identify(B).is_some());
    assert_eq!(disabled.reveal(&a, &store).unwrap(), A);
    apply(&db, &store, Operation::Remove { key_id: b });
    assert!(!Registry::load(&db).unwrap().has_enabled());
    apply(
        &db,
        &store,
        Operation::SetEnabled {
            key_id: a,
            enabled: true,
        },
    );
    assert!(Registry::load(&db).unwrap().identify(A).is_some());
}
#[test]
fn rejects_duplicates_invalid_inputs_and_stale_versions_without_creating_files() {
    let db = Database::memory().unwrap();
    let store = Store::default();
    add(&db, &store, "Primary", A);
    for op in [
        Operation::Add {
            name: "primary".into(),
            value: B.into(),
        },
        Operation::Add {
            name: "New".into(),
            value: A.into(),
        },
        Operation::Add {
            name: "".into(),
            value: B.into(),
        },
        Operation::Add {
            name: "n".repeat(61),
            value: B.into(),
        },
        Operation::Add {
            name: "test".into(),
            value: "short".into(),
        },
        Operation::Add {
            name: "test".into(),
            value: "with space ".repeat(4),
        },
        Operation::Rename {
            key_id: "missing".into(),
            name: "X".into(),
        },
    ] {
        let current = Registry::load(&db).unwrap();
        let revision = current.revision;
        assert!(current
            .change(revision, op, &store, |_| panic!("invalid write"))
            .is_err());
        assert_eq!(store.values.borrow().len(), 1);
    }
    assert!(Registry::load(&db)
        .unwrap()
        .change(
            0,
            Operation::Add {
                name: "new".into(),
                value: B.into()
            },
            &store,
            |_| panic!("stale write")
        )
        .is_err());
}
#[test]
fn failed_file_or_registry_write_keeps_old_credentials_and_cleans_only_fresh_files() {
    let db = Database::memory().unwrap();
    let store = Store::default();
    let a = add(&db, &store, "A", A);
    let current = Registry::load(&db).unwrap();
    let revision = current.revision;
    assert!(current
        .clone()
        .change(
            revision,
            Operation::Add {
                name: "B".into(),
                value: B.into()
            },
            &Store {
                fail_set: true,
                ..Default::default()
            },
            |_| panic!("must not persist after failed file write")
        )
        .is_err());
    assert!(current
        .change(
            revision,
            Operation::Add {
                name: "B".into(),
                value: B.into()
            },
            &store,
            |_| Err("db failed".into())
        )
        .is_err());
    assert_eq!(store.values.borrow().len(), 1);
    assert_eq!(Registry::load(&db).unwrap().reveal(&a, &store).unwrap(), A);
    let id = Registry::load(&db).unwrap().items[0]
        .secret_ref
        .clone()
        .unwrap();
    store.values.borrow_mut().insert(id, B.into());
    assert!(Registry::load(&db).unwrap().reveal(&a, &store).is_err());
}
#[test]
fn failed_cleanup_does_not_restore_deleted_credentials_or_hide_the_warning() {
    let db = Database::memory().unwrap();
    let mut store = Store::default();
    let id = add(&db, &store, "A", A);
    store.fail_delete = true;
    let result = apply(&db, &store, Operation::Remove { key_id: id });
    assert_eq!(result.warnings.len(), 1);
    assert!(Registry::load(&db).unwrap().identify(A).is_none());
}
#[test]
fn registry_capacity_is_bounded_without_losing_existing_keys() {
    let db = Database::memory().unwrap();
    let store = Store::default();
    for index in 0..64 {
        add(
            &db,
            &store,
            &format!("Key {index}"),
            &format!("fixture-generated-{index:04}-0123456789abcdef0123456789"),
        );
    }
    let registry = Registry::load(&db).unwrap();
    let revision = registry.revision;
    assert!(registry
        .change(
            revision,
            Operation::Add {
                name: "Overflow".into(),
                value: A.into()
            },
            &store,
            |_| panic!("capacity must not write")
        )
        .is_err());
    assert_eq!(store.values.borrow().len(), 64);
    assert_eq!(Registry::load(&db).unwrap().view().items.len(), 64);
}

#[test]
fn corrupt_registry_never_falls_back_to_legacy_or_grants_access() {
    let db = Database::memory().unwrap();
    db.set_setting(REGISTRY_KEY, "not-json").unwrap();
    assert!(Registry::load(&db).is_err());
}
