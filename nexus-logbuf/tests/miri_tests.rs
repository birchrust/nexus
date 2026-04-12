//! Miri tests for nexus-logbuf byte ring buffers.
//!
//! Run: `MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test -p nexus-logbuf --test miri_tests`
//!
//! `-Zmiri-ignore-leaks` is needed because the shared Arc-based
//! infrastructure may not be fully dropped in all code paths.

use nexus_logbuf::queue::{mpsc, spsc};

#[test]
fn spsc_claim_write_commit_read() {
    let (mut producer, mut consumer) = spsc::new(1024);

    let mut claim = producer.try_claim(10).expect("claim should succeed");
    claim.copy_from_slice(b"helloworld");
    claim.commit();

    let read = consumer.try_claim().expect("should have a record");
    assert_eq!(&*read, b"helloworld");
}

#[test]
fn spsc_multiple_records() {
    let (mut producer, mut consumer) = spsc::new(1024);

    let payloads: &[&[u8]] = &[b"aaa", b"bb", b"ccccc", b"d", b"eeee"];

    for payload in payloads {
        let mut claim = producer
            .try_claim(payload.len())
            .expect("claim should succeed");
        claim.copy_from_slice(payload);
        claim.commit();
    }

    for payload in payloads {
        let read = consumer.try_claim().expect("should have a record");
        assert_eq!(&*read, *payload);
        drop(read);
    }
}

#[test]
fn mpsc_claim_write_commit_read() {
    let (mut producer, mut consumer) = mpsc::new(1024);

    let mut claim = producer.try_claim(10).expect("claim should succeed");
    claim.copy_from_slice(b"helloworld");
    claim.commit();

    let read = consumer.try_claim().expect("should have a record");
    assert_eq!(&*read, b"helloworld");
}

#[test]
fn spsc_claim_abort() {
    let (mut producer, mut consumer) = spsc::new(1024);

    // Claim and drop without committing (abort).
    let claim = producer.try_claim(16).expect("claim should succeed");
    drop(claim);

    // Write a real record after the aborted one.
    let mut claim = producer.try_claim(5).expect("claim should succeed");
    claim.copy_from_slice(b"after");
    claim.commit();

    // Consumer should skip the aborted region and read the real record.
    let read = consumer.try_claim().expect("should have a record");
    assert_eq!(&*read, b"after");
}

#[test]
fn mpsc_two_producers() {
    let (producer, mut consumer) = mpsc::new(1024);
    let mut p1 = producer.clone();
    let mut p2 = producer;

    let mut claim = p1.try_claim(6).expect("claim should succeed");
    claim.copy_from_slice(b"fromP1");
    claim.commit();

    let mut claim = p2.try_claim(6).expect("claim should succeed");
    claim.copy_from_slice(b"fromP2");
    claim.commit();

    let mut found = Vec::new();
    while let Some(read) = consumer.try_claim() {
        found.push(read.to_vec());
    }

    assert_eq!(found.len(), 2);
    assert!(found.contains(&b"fromP1".to_vec()));
    assert!(found.contains(&b"fromP2".to_vec()));
}
