use igdl::paths::resolve_output_dir_from;
use tempfile::tempdir;

#[test]
fn uses_override_directory_when_provided() {
    let home = tempdir().unwrap();
    std::fs::create_dir(home.path().join("Videos")).unwrap();
    std::fs::create_dir(home.path().join("Movies")).unwrap();
    let custom_dir = home.path().join("custom").join("instagram");

    let dir = resolve_output_dir_from(Some(custom_dir.clone()), home.path()).unwrap();

    assert_eq!(dir, custom_dir);
}

#[test]
fn creates_default_directory_when_missing() {
    let home = tempdir().unwrap();
    std::fs::create_dir(home.path().join("Videos")).unwrap();

    let dir = resolve_output_dir_from(None, home.path()).unwrap();

    assert!(dir.is_dir());
}

#[test]
fn creates_override_directory_when_missing() {
    let home = tempdir().unwrap();
    let custom_dir = home.path().join("custom").join("instagram");

    let dir = resolve_output_dir_from(Some(custom_dir.clone()), home.path()).unwrap();

    assert_eq!(dir, custom_dir);
    assert!(dir.is_dir());
}
