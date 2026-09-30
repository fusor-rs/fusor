use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct AllocationProbe(Arc<AtomicUsize>);
impl Drop for AllocationProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn leases_retain_transfers_and_running_references() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let codec = Codec::local("pool".into(), "generation".into());
    let original = codec.share(AllocationProbe(Arc::clone(&dropped))).unwrap();
    let packet = original.encode(1024, &codec).unwrap();
    drop(original);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    let received = Shared::<AllocationProbe>::decode(packet, &codec).unwrap();
    let reference = codec.resolve(&received).unwrap();
    drop(received);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(reference);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
#[test]
fn pool_generation_and_type_are_validated_and_failed_replies_release() {
    let codec = Codec::local("one".into(), "generation".into());
    let other = Codec::local("two".into(), "generation".into());
    let value = codec.share(7_u32).unwrap();
    assert_eq!(other.resolve(&value).unwrap_err(), WorkerError::WrongPool);
    let packet = value.encode(1024, &codec).unwrap();
    assert_eq!(
        Shared::<String>::decode(packet, &codec).unwrap_err(),
        WorkerError::SharedTypeMismatch
    );
    codec.discard(value.encode(1024, &codec).unwrap());
    let reference = codec.resolve(&value).unwrap();
    codec.invalidate();
    assert_eq!(codec.resolve(&value).unwrap_err(), WorkerError::StaleShared);
    assert_eq!(*reference, 7);
}

#[test]
fn shared_handles_round_trip_across_compiler_type_names() {
    let local = Codec::local("pool".into(), "generation".into());
    let remote = Codec::remote(Arc::new(RemoteLeases {
        pool: "pool".into(),
        generation: "generation".into(),
        alive: true.into(),
    }));
    let original = local.share(AtomicUsize::new(7)).unwrap();
    let mut packet = original.encode(1024, &local).unwrap();
    let Payload::Shared(id) = &mut packet else {
        panic!("expected a shared handle");
    };
    // Stable and the pinned worker nightly can spell the same type differently.
    id.ty = "a different compiler's name for AtomicUsize".into();
    let received = Shared::<AtomicUsize>::decode(packet, &remote).unwrap();
    let returned =
        Shared::<AtomicUsize>::decode(received.encode(1024, &remote).unwrap(), &local).unwrap();
    let allocation = local.resolve(&returned).unwrap();
    assert!(Arc::ptr_eq(&allocation, &local.resolve(&original).unwrap()));
    assert_eq!(allocation.load(Ordering::SeqCst), 7);
}

#[test]
fn shared_type_validation_uses_the_allocation_and_releases_failed_transfers() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let codec = Codec::local("pool".into(), "generation".into());
    let original = codec.share(AllocationProbe(Arc::clone(&dropped))).unwrap();
    let mut packet = original.encode(1024, &codec).unwrap();
    let Payload::Shared(id) = &mut packet else {
        panic!("expected a shared handle");
    };
    id.ty = std::any::type_name::<String>().into();
    drop(original);
    assert_eq!(
        Shared::<String>::decode(packet, &codec).unwrap_err(),
        WorkerError::SharedTypeMismatch
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
