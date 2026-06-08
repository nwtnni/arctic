//! Record implementation-specific counters and histograms.

use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use ribbit::Unpack as _;

use crate::Key;
use crate::Value;
use crate::concurrent::Smr;
use crate::concurrent::smr::Global as _;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::raw::node;

static RECORD: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "stat")]
thread_local! {
    pub(crate) static THREAD: core::cell::RefCell<Thread> = core::cell::RefCell::new(Thread::default()) ;
}

pub fn process<K: Key, V: Value, S: Smr>(map: &mut crate::concurrent::Map<K, V, S>) -> Process {
    let mut compression = Histogram::default();
    let mut node_3 = Histogram::default();
    let mut node_15 = Histogram::default();
    let mut node_47 = Histogram::default();
    let mut node_256 = Histogram::default();

    map.as_sequential()
        .raw
        .postorder()
        .for_each_internal(|meta, child| {
            let bits = meta.len().bits();
            compression.record((bits >> 3) as u64);

            match child {
                edge::Child::Value(_) => {}
                edge::Child::Node(node) => {
                    let histogram = match node.r#type().unpack() {
                        node::Type::Node3 => &mut node_3,
                        node::Type::Node15 => &mut node_15,
                        node::Type::Node47 => &mut node_47,
                        node::Type::Node256 => &mut node_256,
                    };

                    let children =
                        unsafe { node.entries(false, Unbound::<()>::default(), Unbound::<()>::default()) }
                            .filter(|(_, edge)| {
                                !unsafe { edge.as_ref() }
                                    .load_packed(Ordering::Relaxed)
                                    .is_null()
                            })
                            .count();

                    histogram.record(children as u64);
                }
            }
        });

    Process {
        compression,
        node_3,
        node_15,
        node_47,
        node_256,
        garbage: map.smr().garbage(),
    }
}

#[inline]
pub fn thread() -> Thread {
    #[cfg(feature = "stat")]
    {
        THREAD.with_borrow(|thread| thread.clone())
    }

    #[cfg(not(feature = "stat"))]
    {
        Thread
    }
}

#[inline]
pub fn start() {
    if cfg!(feature = "stat") {
        RECORD.store(true, Ordering::Relaxed);
    }
}

#[inline]
pub fn stop() {
    if cfg!(feature = "stat") {
        RECORD.store(false, Ordering::Relaxed);
    }
}

#[inline]
pub fn reset() {
    #[cfg(feature = "stat")]
    THREAD.with_borrow_mut(|thread| *thread = Thread::default());
}

#[derive(Default)]
#[cfg_attr(feature = "stat", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(not(feature = "stat"), expect(unused))]
pub struct Process {
    compression: Histogram,
    node_3: Histogram,
    node_15: Histogram,
    node_47: Histogram,
    node_256: Histogram,
    garbage: u32,
}

#[derive(Copy, Clone)]
pub(crate) enum Counter {
    InsertPessimistic,
    UpdatePessimistic,

    Retire,
    FreeConflict,
    FreeRetire,
    FreeReclaim,
    FreeDrop,
    HazardMatch,

    Node47Consistent,
    Node47CasSuccess,
    Node47CasFailure,

    EntriesOne,
    EntriesMany,
}

pub(crate) enum Max {
    RetireCache,
}

pub(crate) enum Record {
    Flush,
    FreezePop,
    ReclaimDepth,
    ReclaimAge0,
    ReclaimAge1,
    ReclaimAge2,
    ReclaimAge3,
}

#[cfg(feature = "stat")]
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Thread {
    insert_pessimistic: u64,
    update_pessimistic: u64,

    flush: Histogram,
    retire: u64,
    retire_cache: u64,
    free_conflict: u64,
    free_retire: u64,
    free_reclaim: u64,
    free_drop: u64,
    hazard_match: u64,

    entries_one: u64,
    entries_many: u64,

    node_47_consistent: u64,
    node_47_cas_success: u64,
    node_47_cas_failure: u64,

    freeze_pop: Histogram,

    reclaim_depth: Histogram,

    // Age at reclamation for allocations with n byte prefixes
    reclaim_age_0: Histogram,
    reclaim_age_1: Histogram,
    reclaim_age_2: Histogram,
    reclaim_age_3: Histogram,
}

#[cfg(not(feature = "stat"))]
pub struct Thread;

#[inline]
pub(crate) fn increment<C: Into<Counter>>(_counter: C) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with_borrow_mut(|thread| {
            *match _counter.into() {
                Counter::InsertPessimistic => &mut thread.insert_pessimistic,
                Counter::UpdatePessimistic => &mut thread.update_pessimistic,

                Counter::Retire => &mut thread.retire,
                Counter::FreeConflict => &mut thread.free_conflict,
                Counter::FreeRetire => &mut thread.free_retire,
                Counter::FreeReclaim => &mut thread.free_reclaim,
                Counter::FreeDrop => &mut thread.free_drop,
                Counter::HazardMatch => &mut thread.hazard_match,

                Counter::EntriesOne => &mut thread.entries_one,
                Counter::EntriesMany => &mut thread.entries_many,

                Counter::Node47Consistent => &mut thread.node_47_consistent,
                Counter::Node47CasSuccess => &mut thread.node_47_cas_success,
                Counter::Node47CasFailure => &mut thread.node_47_cas_failure,
            } += 1;
        })
    }
}

#[inline]
pub(crate) fn max(_max: Max, _value: u64) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with_borrow_mut(|thread| {
            let old = match _max {
                Max::RetireCache => &mut thread.retire_cache,
            };
            *old = (*old).max(_value);
        })
    }
}

#[inline]
pub(crate) fn record(_record: Record, _value: u64) {
    #[cfg(feature = "stat")]
    if RECORD.load(Ordering::Relaxed) {
        THREAD.with_borrow_mut(|thread| {
            let old = match _record {
                Record::Flush => &mut thread.flush,
                Record::FreezePop => &mut thread.freeze_pop,
                Record::ReclaimDepth => &mut thread.reclaim_depth,
                Record::ReclaimAge0 => &mut thread.reclaim_age_0,
                Record::ReclaimAge1 => &mut thread.reclaim_age_1,
                Record::ReclaimAge2 => &mut thread.reclaim_age_2,
                Record::ReclaimAge3 => &mut thread.reclaim_age_3,
            };
            old.record(_value);
        })
    }
}

#[derive(Clone)]
struct Histogram {
    #[cfg(feature = "stat")]
    inner: hdrhistogram::Histogram<u64>,
}

impl Histogram {
    fn record(&mut self, _value: u64) {
        #[cfg(feature = "stat")]
        self.inner.record(_value).unwrap();
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            #[cfg(feature = "stat")]
            inner: hdrhistogram::Histogram::new(3).unwrap(),
        }
    }
}

#[cfg(feature = "stat")]
impl serde::Serialize for Histogram {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use hdrhistogram::serialization::Serializer as _;
        use hdrhistogram::serialization::V2DeflateSerializer;
        use serde::ser::Error as _;

        let mut buffer = Vec::new();

        {
            let mut encoder = base64::write::EncoderWriter::new(
                &mut buffer,
                &base64::engine::general_purpose::STANDARD,
            );

            V2DeflateSerializer::new()
                .serialize(&self.inner, &mut encoder)
                .map_err(S::Error::custom)?;
        }

        serializer.serialize_str(str::from_utf8(&buffer).map_err(S::Error::custom)?)
    }
}

#[cfg(feature = "stat")]
impl<'de> serde::Deserialize<'de> for Histogram {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use hdrhistogram::serialization::Deserializer;
        use serde::de::Error as _;

        let mut string = <&'de str>::deserialize(deserializer).map(std::io::Cursor::new)?;
        let mut decoder = base64::read::DecoderReader::new(
            &mut string,
            &base64::engine::general_purpose::STANDARD,
        );

        Ok(Histogram {
            inner: Deserializer::new()
                .deserialize(&mut decoder)
                .map_err(D::Error::custom)?,
        })
    }
}
