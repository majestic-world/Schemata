use schematadb_core::{ErrorKind, read_sql_text, write_text_atomic};

#[tokio::test]
async fn reading_sql_text_strips_the_byte_order_mark_and_rejects_other_encodings() {
    let dir = tempfile::tempdir().unwrap();
    let bom = dir.path().join("bom.sql");
    std::fs::write(&bom, b"\xEF\xBB\xBFSELECT 'Cedilha \xC3\xA7';").unwrap();
    assert_eq!(read_sql_text(&bom).await.unwrap(), "SELECT 'Cedilha ç';");

    let latin1 = dir.path().join("latin1.sql");
    std::fs::write(&latin1, b"SELECT '\xE7';").unwrap();
    assert_eq!(read_sql_text(&latin1).await.unwrap_err().kind, ErrorKind::Validation);
}

#[tokio::test]
async fn atomic_write_replaces_content_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("script.sql");
    std::fs::write(&file, "old contents that are longer").unwrap();
    write_text_atomic(&file, "SELECT 1;").await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 1;");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temporary file may be left behind");
}

#[tokio::test]
async fn failed_atomic_write_leaves_the_destination_untouched() {
    let dir = tempfile::tempdir().unwrap();
    // A directory cannot be replaced by a file, so the final rename fails after the content was written.
    let target = dir.path().join("occupied");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("inside.txt"), "precious").unwrap();
    assert!(write_text_atomic(&target, "SELECT 1;").await.is_err());
    assert!(target.is_dir());
    assert_eq!(std::fs::read_to_string(target.join("inside.txt")).unwrap(), "precious");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "the temporary file must be removed");
}
