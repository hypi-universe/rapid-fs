use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use rapid_fs::{FilesystemVfs, MemoryVfs};
use rapid_fs::vfs::{BoundVfs, Vfs, VfsErr};

pub fn resource_path(path: &str) -> String {
    format!("{}/tests/data/{}", env!("CARGO_MANIFEST_DIR"), path)
}

pub fn read_str_resource(path: &str) -> String {
    fs::read_to_string(resource_path(path)).expect(format!("Error reading test resource {}", path).as_str())
}

#[test]
fn memvfs() {
    let vfs = MemoryVfs {
        root: PathBuf::from("/private/path/to/services"), //cannot be empty, all paths must start with this
        data: HashMap::from([
            (
                "/private/path/to/services/123/versions/v1/schema.xml".to_owned(),
                ("schema.xml").to_owned(),
            ),
            (
                "/private/path/to/services/123/versions/v1/pipeline_register.xml"
                    .to_owned(),
                ("pipeline_register.xml").to_owned(),
            ),
            (
                "/private/path/to/services/123/versions/v1/pipeline2.xml".to_owned(),
                ("pipeline2.xml").to_owned(),
            ),
            (
                "/private/path/to/services/123/versions/v1/pipeline_billing_email.xml"
                    .to_owned(),
                ("pipeline_billing_email.xml").to_owned(),
            ),
            (
                "/private/path/to/services/123/versions/v1/endpoint_subscription.xml"
                    .to_owned(),
                ("endpoint_subscription.xml").to_owned(),
            ),
            (
                "/private/path/to/services/123/versions/v1/table_team_icon.xml".to_owned(),
                ("table_team_icon.xml").to_owned(),
            ),
        ]),
    };
    let mut schema = String::new();
    vfs.read(PathBuf::from("/private/path/to/services/123/versions/v1/schema.xml")).unwrap().read_to_string(&mut schema).unwrap();
    assert_eq!("schema.xml", schema);

    match vfs.read(PathBuf::from("schema.xml")) {
        Ok(_) => {
            panic!("Reading non-existent file produced a value")
        }
        Err(e) => {
            match e {
                VfsErr::FileNotFound(_) => {}
                _ => {
                    panic!("Expected file not found error")
                }
            }
        }
    };
}

#[test]
fn fs_vfs() {
    let vfs = FilesystemVfs::new(resource_path("services"));
    let domain = vfs.read_domain_file("music.apps.hypi.ai").unwrap();
    assert_eq!(123, domain.service_id);
    let vfs = BoundVfs::new(domain, Arc::new(vfs));
    match vfs.read_schema_file("../schema.xml") {
        Ok(_) => {
            panic!("Not allowed to use .. in paths")
        }
        Err(e) => {
            match e {
                VfsErr::DotPathsNotSupported(_) => {}
                _ => {
                    panic!("Expected an error about the .. in the file name")
                }
            }
        }
    }
    match vfs.read_schema_file("schema.xml") {
        Ok(file) => {
            assert_eq!(file,
                       r#"<?xml version="1.0"?>
<document>
    <db label="db1" type="mekadb" db_name="abc123" username="user1" password="pass1" host="mekadb.hypi.app" port="2024"/>
    <apis>
        <pipeline import="pipeline_register.xml"/>
        </rest>
    </apis>
</document>
"#
            )
        }
        Err(e) => {
            panic!("Ahhh...this one should've worked! {}", e)
        }
    }
    assert_eq!(vfs.resource_dir().unwrap(), PathBuf::from(resource_path("services/123/files")));
    assert!(vfs.resolve_resource(PathBuf::from("../domains/music.apps.hypi.ai")).is_err());
    assert_eq!(
        fs::read_to_string(vfs.resolve_resource("file1.txt".into()).unwrap()).unwrap(),
        "file1 content\n"
    );
}
