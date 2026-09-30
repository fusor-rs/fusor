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
