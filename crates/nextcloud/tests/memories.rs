use std::sync::Arc;

use craft_nextcloud::testing::{FakeServer, Fault};
use craft_nextcloud::{Client, Error};
use serde_json::json;

fn client(server: &FakeServer) -> Client {
    Client::new(server.account(), FakeServer::PASSWORD.into(), Arc::new(rustls_rustcrypto::provider())).unwrap()
}

#[test]
fn memories_absent_is_none() {
    let server = FakeServer::start();
    assert!(client(&server).memories().unwrap().is_none());
}

#[test]
fn days_returns_photos_across_batches() {
    let server = FakeServer::start();
    server.enable_memories();
    let mut ids = Vec::new();
    for i in 0..250u32 {
        let id = server.add_file(&format!("/P/{i}.jpg"), vec![0; i as usize + 1]);
        server.set_photo(&format!("/P/{i}.jpg"), 19000 + i64::from(i), 400 + i, 300 + i, 1_700_000_000 + i64::from(i), json!({}));
        ids.push(id);
    }
    let client = client(&server);
    let memories = client.memories().unwrap().unwrap();
    let photos = memories.days().unwrap();
    assert_eq!(photos.len(), 250);
    for (i, (p, id)) in photos.iter().zip(&ids).enumerate() {
        assert_eq!(p.fileid, *id);
        assert_eq!((p.w, p.h), (400 + i as u32, 300 + i as u32));
        assert_eq!(p.basename, format!("{i}.jpg"));
        assert_eq!(p.size, i as u64 + 1);
        assert!(!p.etag.is_empty());
    }
    assert_eq!(server.requests(), 1 + 1 + 3);
}

#[test]
fn empty_days_makes_no_batch_request() {
    let server = FakeServer::start();
    server.enable_memories();
    let client = client(&server);
    assert!(client.memories().unwrap().unwrap().days().unwrap().is_empty());
    assert_eq!(server.requests(), 2);
}

#[test]
fn string_numbers_are_accepted() {
    let server = FakeServer::start();
    server.enable_memories();
    server.add_file("/a.jpg", vec![1]);
    server.set_photo("/a.jpg", 19000, 10, 20, 1_719_571_872, json!({}));
    let client = client(&server);
    let photos = client.memories().unwrap().unwrap().days().unwrap();
    assert_eq!(photos[0].epoch, Some(1_719_571_872));
}

#[test]
fn info_maps_exif() {
    let server = FakeServer::start();
    server.enable_memories();
    let id = server.add_file("/a.jpg", vec![1]);
    let exif = json!({"Make":"SONY","Model":"ILCE-7RM6","LensModel":"FE 24-70mm F2.8 GM II","ExposureTime":"1/250","FNumber":2.8,"ISO":100,"FocalLength":"35","Orientation":1});
    server.set_photo("/a.jpg", 19000, 7000, 4000, 1_719_571_872, exif);
    let client = client(&server);
    let meta = client.memories().unwrap().unwrap().info(id).unwrap();
    assert_eq!(meta.make.as_deref(), Some("SONY"));
    assert_eq!(meta.model.as_deref(), Some("ILCE-7RM6"));
    assert_eq!(meta.lens.as_deref(), Some("FE 24-70mm F2.8 GM II"));
    assert_eq!(meta.exposure_time, Some(0.004));
    assert_eq!(meta.f_number, Some(2.8));
    assert_eq!(meta.iso, Some(100));
    assert_eq!(meta.focal_length, Some(35.0));
    assert_eq!(meta.orientation, Some(1));
    assert_eq!(meta.captured, Some(1_719_571_872));
    assert_eq!((meta.width, meta.height), (Some(7000), Some(4000)));
}

#[test]
fn malformed_info() {
    let server = FakeServer::start();
    server.enable_memories();
    let id = server.add_file("/a.jpg", vec![1]);
    server.set_photo("/a.jpg", 19000, 1, 1, 1, json!({}));
    let client = client(&server);
    let memories = client.memories().unwrap().unwrap();
    server.fail_next(Fault::Garbage);
    assert!(matches!(memories.info(id), Err(Error::Malformed(_))));
}

#[test]
fn non_json_describe_is_none() {
    let server = FakeServer::start();
    server.enable_memories();
    server.fail_next(Fault::Garbage);
    assert!(client(&server).memories().unwrap().is_none());
}
