//! The `isthmus-hid` thread: one HID event-system client, its services,
//! hot-plug, wake, and every property write with its journal.
//!
//! **Why one thread with its own run loop.** IOKit delivers matching,
//! removal, property-changed and power notifications to a run loop, and
//! the client and its service objects are not thread-safe. So one thread
//! owns all of them, state lives in thread-locals, and other threads talk
//! to it only by queueing a [`Command`] and waking the run loop with
//! `CFRunLoopPerformBlock`, which is thread-safe.
//!
//! **Which client.** The private Passive client
//! (`IOHIDEventSystemClientCreateWithType(.., 2, ..)`) sees and sets every
//! pointer key without an entitlement; the public simple client cannot see
//! `HIDUseLinearScalingMouseAcceleration` or `SerialNumber`. Private symbols
//! are looked up with `dlsym`, so a system without them falls back to the
//! simple client ([`HidBackend::Limited`]) and to polling for hot-plug, as
//! does one whose Passive client misses keyboards or pointers the simple
//! client lists. A Passive client that lists nothing because nothing is
//! attached yet is kept.
//!
//! **Callbacks never touch state directly.** Each pushes an event onto a
//! thread-local queue and pumps it; a callback that arrives while the
//! state is busy (re-entered from inside an IOKit call) is handled when the
//! busy pump finishes. Every callback is wrapped in `catch_unwind`: a panic
//! must not unwind into IOKit.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::ptr::{self, NonNull};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use block2::RcBlock;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFArray, CFDictionary, CFNumber, CFRetained, CFRunLoop,
    CFRunLoopRunResult, CFRunLoopTimer, CFString, CFType, kCFRunLoopDefaultMode,
};
use objc2_io_kit::{
    IOAllowPowerChange, IODeregisterForSystemPower, IOHIDEventSystemClient, IOHIDServiceClient,
    IONotificationPort, IONotificationPortRef, IOObjectRelease, IORegisterForSystemPower,
    IORegistryEntryCreateCFProperty, IORegistryEntryIDMatching, IOServiceClose,
    IOServiceGetMatchingService, io_connect_t, io_object_t, io_service_t,
};
use parking_lot::Mutex;
use tracing::{debug, error, info, warn};

use super::cf::{self, CfValue, RunLoopHandle};
use super::devices::{self, Desired, ExecReport, JournalStore, Observed, PropKey, PropertyIo};
use crate::device::{self, HidService};
use crate::error::Error;
use crate::journal::{self, PropertyValue};
use crate::service::HidSummary;
use crate::status::HidBackend;

/// The private client type that may read and set service properties
/// without an entitlement.
const CLIENT_TYPE_PASSIVE: i32 = 2;

/// The Generic Desktop usage page, where keyboards and mice live.
const PAGE_GENERIC_DESKTOP: u32 = 0x01;
/// Generic Desktop Pointer: what some mice and every trackpad conform to.
const USAGE_POINTER: u32 = 0x01;
/// Generic Desktop Mouse.
const USAGE_MOUSE: u32 = 0x02;
/// Generic Desktop Keyboard; a keyboard's consumer-control service lists it
/// among its usage pairs too, so it gets the same mapping.
const USAGE_KEYBOARD: u32 = 0x06;

/// How long hot-plug bursts settle before a reconcile: a KVM switch
/// removes and adds several services within a few hundred milliseconds.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(300);
/// How long after wake devices settle before a reconcile.
pub(crate) const AFTER_WAKE: Duration = Duration::from_millis(1500);
/// The rescan period when hot-plug notifications are unavailable.
const POLL: Duration = Duration::from_secs(2);
/// The rescan period even with hot-plug notifications: whether they fire
/// for a Passive client on a KVM switch is unverified, and a `hidd` restart
/// sends none. A rescan only compares registry ids, so it is cheap.
const SAFETY_POLL: Duration = Duration::from_secs(10);
/// A timer interval that never comes around: the debounce timer is moved by
/// hand and otherwise sleeps.
const NEVER: f64 = 1.0e9;

/// `kIOMessageCanSystemSleep` (`IOMessage.h`): idle sleep asks first, and
/// waits for an answer.
const CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
/// `kIOMessageSystemWillSleep`: sleep is coming, and waits for an answer.
const SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
/// `kIOMessageSystemHasPoweredOn`: awake again; devices may have been
/// re-enumerated.
const SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

/// The global keys whose change (System Settings, another tool) overwrites
/// every service's value, and the keys of Isthmus's that then need a
/// rewrite even when they cannot be read back.
const WATCHED: &[(&str, &[PropKey])] = &[
    ("HIDMouseAcceleration", &[PropKey::MouseAcceleration]),
    (
        "HIDUseLinearScalingMouseAcceleration",
        &[PropKey::LinearScaling, PropKey::MouseAcceleration],
    ),
    ("HIDPointerResolution", &[]),
    ("HIDFKeyMode", &[PropKey::FKeyMode]),
];

/// `IOHIDEventSystemClientCreateWithType(allocator, type, attributes)`.
type CreateWithTypeFn =
    unsafe extern "C" fn(*const c_void, i32, *const c_void) -> *mut IOHIDEventSystemClient;
/// `IOHIDServiceClientCallback(target, refcon, service)` (`hidobjcbase.h`).
type ServiceCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut IOHIDServiceClient);
/// `IOHIDEventSystemClientRegisterDeviceMatchingCallback`.
type RegisterMatchingFn =
    unsafe extern "C" fn(*mut IOHIDEventSystemClient, ServiceCallback, *mut c_void, *mut c_void);
/// `IOHIDServiceClientRegisterRemovalCallback`.
type RegisterRemovalFn =
    unsafe extern "C" fn(*mut IOHIDServiceClient, ServiceCallback, *mut c_void, *mut c_void);
/// `IOHIDEventSystemClientPropertyChangedCallback(target, context, key, value)`.
type PropertyCallback =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const CFString, *const CFType);
/// `IOHIDEventSystemClientRegisterPropertyChangedCallback`.
type RegisterPropertyFn = unsafe extern "C" fn(
    *mut IOHIDEventSystemClient,
    *const CFString,
    PropertyCallback,
    *mut c_void,
    *mut c_void,
);
/// `IOHIDEventSystemClientScheduleWithRunLoop(client, run loop, mode)`.
type ScheduleFn =
    unsafe extern "C" fn(*mut IOHIDEventSystemClient, *const CFRunLoop, *const CFString);
/// `IOHIDEventSystemClientSetMatchingMultiple(client, array of criteria)`.
type SetMatchingMultipleFn = unsafe extern "C" fn(*mut IOHIDEventSystemClient, *const CFArray);

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Declared here because `objc2-io-kit` panics when it returns NULL.
    fn IOHIDEventSystemClientCreateSimpleClient(
        allocator: *const c_void,
    ) -> *mut IOHIDEventSystemClient;
    /// Declared here because `objc2-io-kit` panics when it returns NULL.
    fn IOHIDServiceClientGetRegistryID(service: *mut IOHIDServiceClient) -> *const CFType;
}

/// The private event-system functions, where this system exports them.
/// Each is optional: a missing one turns off only what needs it.
#[derive(Clone, Copy)]
struct Spi {
    create_with_type: Option<CreateWithTypeFn>,
    register_matching: Option<RegisterMatchingFn>,
    register_removal: Option<RegisterRemovalFn>,
    register_property: Option<RegisterPropertyFn>,
    schedule: Option<ScheduleFn>,
    set_matching_multiple: Option<SetMatchingMultipleFn>,
}

impl Spi {
    /// Looks every function up once.
    fn load() -> Spi {
        // SAFETY: each symbol, where present, has the C signature Apple's
        // own IOKit headers (IOHIDEventSystemClientPrivate.h,
        // hidobjcbase.h) give it, which these function types spell out.
        unsafe {
            Spi {
                create_with_type: cf::function(c"IOHIDEventSystemClientCreateWithType")
                    .map(|p| std::mem::transmute::<*mut c_void, CreateWithTypeFn>(p.as_ptr())),
                register_matching: cf::function(
                    c"IOHIDEventSystemClientRegisterDeviceMatchingCallback",
                )
                .map(|p| std::mem::transmute::<*mut c_void, RegisterMatchingFn>(p.as_ptr())),
                register_removal: cf::function(c"IOHIDServiceClientRegisterRemovalCallback")
                    .map(|p| std::mem::transmute::<*mut c_void, RegisterRemovalFn>(p.as_ptr())),
                register_property: cf::function(
                    c"IOHIDEventSystemClientRegisterPropertyChangedCallback",
                )
                .map(|p| std::mem::transmute::<*mut c_void, RegisterPropertyFn>(p.as_ptr())),
                schedule: cf::function(c"IOHIDEventSystemClientScheduleWithRunLoop")
                    .map(|p| std::mem::transmute::<*mut c_void, ScheduleFn>(p.as_ptr())),
                set_matching_multiple: cf::function(c"IOHIDEventSystemClientSetMatchingMultiple")
                    .map(|p| std::mem::transmute::<*mut c_void, SetMatchingMultipleFn>(p.as_ptr())),
            }
        }
    }
}

/// One HID event-system client.
pub(crate) struct HidClient {
    client: CFRetained<IOHIDEventSystemClient>,
    /// Full for the Passive client, Limited for the public simple one.
    backend: HidBackend,
    /// The private functions this system has, for hot-plug and matching.
    spi: Spi,
}

impl HidClient {
    /// The Passive client, else the public simple one; `None` when the HID
    /// event system refuses both.
    ///
    /// The client is opened once per session, so a wrong fallback lasts until
    /// Isthmus restarts. A Passive client that lists nothing is not
    /// necessarily broken: the event system answers a match with no services
    /// with NULL rather than an empty list, which is what happens when no
    /// keyboard or mouse is attached at start (a desktop Mac whose KVM
    /// points at another computer). See [`keeps_passive`].
    pub(crate) fn open() -> Option<HidClient> {
        let spi = Spi::load();
        let Some(passive) = HidClient::open_passive(spi) else {
            info!("the private HID client is unavailable; pointer speed and hot-plug are limited");
            return HidClient::open_simple(spi);
        };
        let passive_lists = passive.client.services().is_some();
        // The simple client is asked only to judge a Passive client that
        // lists nothing, and only reads.
        let simple = if passive_lists {
            None
        } else {
            HidClient::open_simple(spi)
        };
        let simple_lists_input = simple.as_ref().is_some_and(HidClient::lists_input);
        if keeps_passive(passive_lists, simple_lists_input) {
            if !passive_lists {
                info!("no keyboard or pointer is attached yet; keeping the Passive HID client");
            }
            return Some(passive);
        }
        warn!(
            "the Passive HID client misses keyboards or pointers the simple client lists; \
             using the simple client, with pointer speed and hot-plug limited"
        );
        simple
    }

    /// The Passive client, narrowed to keyboards and pointers; `None` when
    /// this system lacks it or the event system refuses it.
    fn open_passive(spi: Spi) -> Option<HidClient> {
        let create = spi.create_with_type?;
        // SAFETY: NULL allocator and attributes are the documented
        // defaults; the result is +1 or NULL.
        let raw = NonNull::new(unsafe { create(ptr::null(), CLIENT_TYPE_PASSIVE, ptr::null()) })?;
        let opened = HidClient {
            // SAFETY: a +1 reference from a Create function.
            client: unsafe { CFRetained::from_raw(raw) },
            backend: HidBackend::Full,
            spi,
        };
        opened.match_keyboards_and_pointers();
        Some(opened)
    }

    /// The public simple client: every system has it, but it sees neither
    /// serial numbers nor the linear pointer flag.
    fn open_simple(spi: Spi) -> Option<HidClient> {
        // SAFETY: a NULL allocator is the default; the result is +1 or NULL.
        let raw = NonNull::new(unsafe { IOHIDEventSystemClientCreateSimpleClient(ptr::null()) })?;
        Some(HidClient {
            // SAFETY: a +1 reference from a Create function.
            client: unsafe { CFRetained::from_raw(raw) },
            backend: HidBackend::Limited,
            spi,
        })
    }

    /// Whether this client lists a keyboard, mouse or pointer service: what
    /// tells a Passive client that cannot see them from one that has
    /// nothing to see.
    fn lists_input(&self) -> bool {
        self.services().iter().any(|(_, service)| {
            [USAGE_KEYBOARD, USAGE_MOUSE, USAGE_POINTER]
                .into_iter()
                .any(|usage| service.conforms_to(PAGE_GENERIC_DESKTOP, usage))
        })
    }

    /// The client as the private C functions take it.
    fn raw(&self) -> *mut IOHIDEventSystemClient {
        CFRetained::as_ptr(&self.client).as_ptr()
    }

    /// Which client this is, for the status and for what it can set.
    pub(crate) fn backend(&self) -> HidBackend {
        self.backend
    }

    /// Hot-plug notifications need the private matching, removal and
    /// scheduling functions, and the Passive client.
    fn hot_plug(&self) -> bool {
        self.backend == HidBackend::Full
            && self.spi.register_matching.is_some()
            && self.spi.register_removal.is_some()
            && self.spi.schedule.is_some()
    }

    /// Narrows the client to keyboards, mice and pointers, so listing and
    /// matching skip the hundred-odd other services (audio, sensors).
    fn match_keyboards_and_pointers(&self) {
        let Some(set_matching) = self.spi.set_matching_multiple else {
            return;
        };
        let (page_key, usage_key) = (cf::string("DeviceUsagePage"), cf::string("DeviceUsage"));
        let page = CFNumber::new_i32(PAGE_GENERIC_DESKTOP.cast_signed());
        let criteria: Vec<CFRetained<CFDictionary<CFString, CFNumber>>> =
            [USAGE_KEYBOARD, USAGE_MOUSE, USAGE_POINTER]
                .into_iter()
                .map(|usage| {
                    let usage = CFNumber::new_i32(usage.cast_signed());
                    CFDictionary::from_slices(&[&*page_key, &*usage_key], &[&*page, &*usage])
                })
                .collect();
        let array = CFArray::from_retained_objects(&criteria);
        // SAFETY: the client is valid and the array holds matching
        // dictionaries; the client copies what it keeps.
        unsafe { set_matching(self.raw(), CFRetained::as_ptr(&array).as_ptr().cast()) };
    }

    /// Every service the client sees, with its registry id.
    pub(crate) fn services(&self) -> Vec<(u64, CFRetained<IOHIDServiceClient>)> {
        let Some(array) = self.client.services() else {
            return Vec::new();
        };
        // SAFETY: `IOHIDEventSystemClientCopyServices` returns an array of
        // `IOHIDServiceClientRef`.
        let typed: CFRetained<CFArray<IOHIDServiceClient>> =
            unsafe { CFRetained::cast_unchecked(array) };
        let mut out: Vec<(u64, CFRetained<IOHIDServiceClient>)> = typed
            .iter()
            .filter_map(|service| registry_id(&service).map(|rid| (rid, service)))
            .collect();
        out.sort_by_key(|(rid, _)| *rid);
        out
    }

    /// The system-wide value of a numeric key.
    pub(crate) fn global(&self, key: &str) -> Option<i64> {
        self.client
            .property(&cf::string(key))
            .and_then(|value| cf::as_i64(&value))
    }
}

/// Whether a Passive client is kept: when it lists services, and also when
/// it lists none and the simple client sees no keyboard or pointer either,
/// because then nothing is attached yet and the Passive client will see the
/// devices, with hot-plug, serial numbers and linear scaling, once they
/// come. Only a Passive client blind to input the simple client sees is
/// given up.
pub(crate) fn keeps_passive(passive_lists_services: bool, simple_lists_input: bool) -> bool {
    passive_lists_services || !simple_lists_input
}

/// A service's registry id, which the event-tap callback also sees as the
/// sender of each event (CGEvent field 87).
fn registry_id(service: &IOHIDServiceClient) -> Option<u64> {
    let raw = ptr::from_ref(service).cast_mut();
    // SAFETY: a valid service; the result is +0 (Get rule) or NULL.
    let value = unsafe { IOHIDServiceClientGetRegistryID(raw) };
    // SAFETY: a +0 CF object owned by the service, used before it returns.
    let value = unsafe { value.as_ref() }?;
    cf::as_i64(value).map(i64::cast_unsigned)
}

/// Reads what Isthmus needs to know about one service.
pub(crate) fn read_service(service: &IOHIDServiceClient, registry_id: u64) -> HidService {
    let text = |key: &str| {
        service
            .property(&cf::string(key))
            .and_then(|v| cf::as_string(&v))
            .unwrap_or_default()
    };
    let int = |key: &str| {
        service
            .property(&cf::string(key))
            .and_then(|v| cf::as_i64(&v))
    };
    let u32_of = |key: &str| int(key).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    HidService {
        registry_id,
        vendor_id: u32_of("VendorID"),
        product_id: u32_of("ProductID"),
        location_id: u32_of("LocationID"),
        product: text("Product"),
        manufacturer: text("Manufacturer"),
        serial_number: text("SerialNumber"),
        transport: text("Transport"),
        built_in: int("Built-In").is_some_and(|v| v != 0),
        primary_usage_page: u32_of("PrimaryUsagePage"),
        primary_usage: u32_of("PrimaryUsage"),
        is_keyboard: service.conforms_to(PAGE_GENERIC_DESKTOP, USAGE_KEYBOARD),
        is_pointer: service.conforms_to(PAGE_GENERIC_DESKTOP, USAGE_MOUSE)
            || service.conforms_to(PAGE_GENERIC_DESKTOP, USAGE_POINTER),
        scroll_acceleration_type: text("HIDScrollAccelerationType"),
        pointer_acceleration_type: text("HIDPointerAccelerationType"),
    }
}

/// The driver's `HIDEventServiceProperties` dictionary in the IORegistry,
/// where values pushed to a service land. The event system does not hand
/// every key back through `IOHIDServiceClientCopyProperty` (the keyboard
/// filter returns neither `UserKeyMapping` nor the System Settings modifier
/// map), so this is the second place a read looks.
fn registry_properties(registry_id: u64) -> Option<CFRetained<CFDictionary>> {
    // SAFETY: builds a matching dictionary; nothing else.
    let matching = unsafe { IORegistryEntryIDMatching(registry_id) }?;
    // SAFETY: a mutable dictionary is a dictionary.
    let matching: CFRetained<CFDictionary> = unsafe { CFRetained::cast_unchecked(matching) };
    // SAFETY: port 0 is the default main port; the dictionary is consumed.
    let entry = unsafe { IOServiceGetMatchingService(0, Some(matching)) };
    if entry == 0 {
        return None;
    }
    let key = cf::string("HIDEventServiceProperties");
    // SAFETY: a valid registry entry and key; the result is +1 or NULL.
    let value = unsafe { IORegistryEntryCreateCFProperty(entry, Some(&key), None, 0) };
    let _ = IOObjectRelease(entry);
    value?.downcast::<CFDictionary>().ok()
}

/// Reads one property: the event system's answer first, the registry's
/// when the event system gives none.
fn read_property(
    service: &IOHIDServiceClient,
    registry: &mut Option<Option<CFRetained<CFDictionary>>>,
    registry_id: u64,
    key: &str,
) -> Option<PropertyValue> {
    let key_string = cf::string(key);
    if let Some(value) = service.property(&key_string) {
        return cf::property_value(key, &value);
    }
    let dictionary = registry.get_or_insert_with(|| registry_properties(registry_id));
    let value = cf::dictionary_value(dictionary.as_ref()?, &key_string)?;
    cf::property_value(key, &value)
}

/// What reconciling needs to know about one service now.
fn observe(service: &IOHIDServiceClient, info: &HidService, backend: HidBackend) -> Observed {
    let rid = info.registry_id;
    let mut registry = None;
    let mut values = BTreeMap::new();
    let mut keys = Vec::new();
    if info.is_keyboard {
        keys.extend([PropKey::UserKeyMapping, PropKey::FKeyMode]);
    }
    if info.is_pointer {
        keys.extend([PropKey::LinearScaling, PropKey::MouseAcceleration]);
    }
    for key in keys {
        if let Some(value) = read_property(service, &mut registry, rid, key.name()) {
            values.insert(key, value);
        }
    }
    // Only the event system's own answer says the service supports linear
    // scaling: the registry carries a copy of every global on every service.
    let publishes_linear = info.is_pointer
        && backend == HidBackend::Full
        && service
            .property(&cf::string(PropKey::LinearScaling.name()))
            .is_some();
    let system_modifier_keys = info.is_keyboard
        && matches!(
            read_property(service, &mut registry, rid, "HIDKeyboardModifierMappingPairs"),
            Some(PropertyValue::KeyMapping(pairs)) if !pairs.is_empty()
        );
    Observed {
        service: info.clone(),
        values,
        publishes_linear,
        system_modifier_keys,
    }
}

/// A service being tracked: the object to write through, and what was
/// read from it when it appeared (identity and kind do not change).
struct Tracked {
    service: CFRetained<IOHIDServiceClient>,
    info: HidService,
}

/// Property reads and writes on the tracked services, through the client
/// that sees them.
struct LiveIo<'a> {
    client: &'a HidClient,
    tracked: &'a BTreeMap<u64, Tracked>,
}

impl PropertyIo for LiveIo<'_> {
    /// Verification asks the event system only: the registry copy may lag
    /// a write, and a stale copy must not make a good write look failed.
    fn read(&mut self, registry_id: u64, key: &str) -> Option<PropertyValue> {
        let tracked = self.tracked.get(&registry_id)?;
        let value = tracked.service.property(&cf::string(key))?;
        cf::property_value(key, &value)
    }

    fn write(&mut self, registry_id: u64, key: &str, value: &PropertyValue) -> bool {
        let (Some(tracked), Some(value)) = (self.tracked.get(&registry_id), CfValue::of(value))
        else {
            return false;
        };
        // SAFETY: numbers are CFNumbers and key mappings arrays of
        // source/destination dictionaries of 64-bit CFNumbers: the types the
        // HID filters read these keys as.
        unsafe {
            tracked
                .service
                .set_property(&cf::string(key), value.as_type())
        }
    }

    fn global(&mut self, key: &str) -> Option<i64> {
        self.client.global(key)
    }
}

/// Every keyboard, mouse and trackpad service, read once by a client of its
/// own. Reads only.
pub(crate) fn list_services() -> Result<Vec<HidService>, Error> {
    let client = HidClient::open().ok_or(Error::Unsupported {
        what: "the HID event system",
    })?;
    Ok(client
        .services()
        .into_iter()
        .map(|(rid, service)| read_service(&service, rid))
        .filter(|s| s.is_keyboard || s.is_pointer)
        .collect())
}

/// Puts back everything `store` journals, with a client of its own and no
/// thread: the CLI's recovery after a crash.
pub(crate) fn recover_once(store: &mut JournalStore) -> Result<ExecReport, Error> {
    let client = HidClient::open().ok_or(Error::Unsupported {
        what: "the HID event system",
    })?;
    let tracked: BTreeMap<u64, Tracked> = client
        .services()
        .into_iter()
        .map(|(rid, service)| {
            let info = read_service(&service, rid);
            (rid, Tracked { service, info })
        })
        .collect();
    let observed = tracked
        .iter()
        .map(|(rid, t)| (*rid, observe(&t.service, &t.info, client.backend)))
        .collect();
    let steps = devices::restore_everything(&observed, &store.journal);
    let mut io = LiveIo {
        client: &client,
        tracked: &tracked,
    };
    Ok(devices::execute(steps, &mut io, store, journal::recover))
}

/// A request to the HID thread.
pub(crate) enum Command {
    /// Make this true; reconciles at once when it changed.
    Desired(Box<Desired>),
    /// Reconcile once things have been quiet for this long.
    ReconcileAfter(Duration),
    /// Switch off and put everything back now; replies when done.
    RestoreAll(SyncSender<ExecReport>),
    Stop,
}

/// What happened on the HID thread's run loop.
enum Event {
    /// Commands are queued.
    Commands,
    /// A service appeared (hot-plug, a KVM switch).
    Matched,
    /// The service with this registry id went away, with its properties.
    Removed(u64),
    /// A global property changed: someone pushed it to every service.
    PropertyChanged(Option<String>),
    /// The debounce timer fired.
    Timer,
    /// Time to rescan for services that came or went unannounced.
    Poll,
    /// The Mac woke up.
    PoweredOn,
}

thread_local! {
    /// The HID thread's state; `None` on every other thread.
    static STATE: RefCell<Option<HidState>> = const { RefCell::new(None) };
    /// Events waiting for the state to be free.
    static EVENTS: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
    /// The power-management connection, for acknowledging sleep from inside
    /// the power callback without touching the state.
    static ROOT_PORT: Cell<io_connect_t> = const { Cell::new(0) };
    /// Set by [`Command::Stop`]: the run loop's last turn.
    static STOPPING: Cell<bool> = const { Cell::new(false) };
}

/// Queues `event` and handles every queued event, unless the state is busy
/// further up this thread's stack, whose pump then handles it.
fn post(event: Event) {
    EVENTS.with(|queue| queue.borrow_mut().push_back(event));
    STATE.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(state) = guard.as_mut() else {
            EVENTS.with(|queue| queue.borrow_mut().clear());
            return;
        };
        while let Some(event) = EVENTS.with(|queue| queue.borrow_mut().pop_front()) {
            state.handle(event);
        }
    });
}

/// Runs a callback body so that a panic stops here, not inside IOKit.
fn guarded(what: &str, body: impl FnOnce()) {
    if std::panic::catch_unwind(AssertUnwindSafe(body)).is_err() {
        error!(callback = what, "a HID callback panicked");
    }
}

/// A service appeared: reconcile once the burst settles. The service itself
/// is picked up by the rescan, which also sees ones that came together.
extern "C" fn on_matched(
    _target: *mut c_void,
    _refcon: *mut c_void,
    _service: *mut IOHIDServiceClient,
) {
    guarded("matching", || post(Event::Matched));
}

/// A service went away; its registry id rides in `refcon`.
extern "C" fn on_removed(
    _target: *mut c_void,
    refcon: *mut c_void,
    _service: *mut IOHIDServiceClient,
) {
    guarded("removal", || post(Event::Removed(refcon.addr() as u64)));
}

/// A global property changed: every service's value was just overwritten.
extern "C" fn on_property(
    _target: *mut c_void,
    _context: *mut c_void,
    property: *const CFString,
    _value: *const CFType,
) {
    guarded("property", || {
        // SAFETY: the event system passes the changed key, valid for the
        // call, or NULL.
        let key = unsafe { property.as_ref() }.map(ToString::to_string);
        post(Event::PropertyChanged(key));
    });
}

/// Sleep and wake. Sleep is acknowledged without touching the state; wake
/// schedules a reconcile once devices settle.
extern "C-unwind" fn on_power(
    _refcon: *mut c_void,
    _service: io_service_t,
    message: u32,
    argument: *mut c_void,
) {
    guarded("power", || match message {
        // Sleep waits up to 30 s for an answer: acknowledge at once.
        CAN_SYSTEM_SLEEP | SYSTEM_WILL_SLEEP => {
            let root = ROOT_PORT.with(Cell::get);
            if root != 0 {
                let _ = IOAllowPowerChange(root, argument.addr().cast_signed());
            }
        }
        SYSTEM_HAS_POWERED_ON => post(Event::PoweredOn),
        _ => {}
    });
}

/// The system power registration, undone on drop.
struct Power {
    /// Delivers power messages to the HID run loop.
    port: IONotificationPortRef,
    /// The root power domain connection, which acknowledges sleep.
    root: io_connect_t,
    /// What deregistration takes back.
    notifier: io_object_t,
}

impl Power {
    /// Registers for sleep and wake on `run_loop`; `None` when refused.
    fn register(run_loop: &CFRunLoop) -> Option<Power> {
        let mut port: IONotificationPortRef = ptr::null_mut();
        let mut notifier: io_object_t = 0;
        // SAFETY: valid out-pointers; the callback is a plain function and
        // the refcon is unused.
        let root = unsafe {
            IORegisterForSystemPower(
                ptr::null_mut(),
                &raw mut port,
                Some(on_power),
                &raw mut notifier,
            )
        };
        if root == 0 || port.is_null() {
            warn!("could not register for sleep and wake notifications");
            return None;
        }
        // SAFETY: the port was just created and is valid.
        let source = unsafe { IONotificationPort::run_loop_source(port) };
        // SAFETY: an extern static CF constant.
        run_loop.add_source(source.as_deref(), unsafe { kCFRunLoopDefaultMode });
        ROOT_PORT.with(|cell| cell.set(root));
        Some(Power {
            port,
            root,
            notifier,
        })
    }
}

impl Drop for Power {
    fn drop(&mut self) {
        ROOT_PORT.with(|cell| cell.set(0));
        // SAFETY: the notifier came from IORegisterForSystemPower and is
        // deregistered once; then the connection and port are released.
        unsafe { IODeregisterForSystemPower(&raw mut self.notifier) };
        let _ = IOServiceClose(self.root);
        // SAFETY: created by IORegisterForSystemPower, destroyed once.
        unsafe { IONotificationPort::destroy(self.port) };
    }
}

/// When a debounced reconcile should run: once things have been quiet for
/// `delay`, and never before a deadline already promised (a wake's 1.5 s is
/// not cut short by a hot-plug's 300 ms).
pub(crate) fn next_deadline(pending: Option<f64>, now: f64, delay: f64) -> f64 {
    pending.map_or(now + delay, |p| p.max(now + delay))
}

/// Everything the HID thread owns.
struct HidState {
    /// `None` when the HID event system refused every client.
    client: Option<HidClient>,
    run_loop: CFRetained<CFRunLoop>,
    commands: Receiver<Command>,
    /// Keyboard and pointer services by registry id.
    tracked: BTreeMap<u64, Tracked>,
    /// Services that are neither keyboards nor pointers, not read again.
    ignored: BTreeSet<u64>,
    journal: JournalStore,
    /// What the service last asked for.
    desired: Desired,
    /// Keys a global push overwrote since the last reconcile.
    force: BTreeSet<PropKey>,
    /// The (service, key) writes the last reconcile could not make: the
    /// next one makes them again even where the value cannot be read back.
    retry: BTreeSet<(u64, &'static str)>,
    /// Fires the debounced reconcile; moved by [`HidState::schedule`].
    debounce: Option<CFRetained<CFRunLoopTimer>>,
    /// When the debounced reconcile is due, if one is.
    deadline: Option<f64>,
    /// The rescan timer; kept alive here.
    _poll: Option<CFRetained<CFRunLoopTimer>>,
    _power: Option<Power>,
    /// The watched property keys, alive as long as the registrations.
    _watched: Vec<CFRetained<CFString>>,
    /// What was last published.
    summary: HidSummary,
    on_summary: Box<dyn Fn(HidSummary) + Send>,
}

impl HidState {
    /// Handles one run-loop event.
    fn handle(&mut self, event: Event) {
        match event {
            Event::Commands => {
                while let Ok(command) = self.commands.try_recv() {
                    self.command(command);
                }
            }
            Event::Matched => self.schedule(DEBOUNCE),
            Event::Removed(rid) => {
                self.tracked.remove(&rid);
                devices::JournalSink::forget_service(&mut self.journal, rid);
                self.schedule(DEBOUNCE);
            }
            Event::PropertyChanged(key) => {
                if let Some((_, keys)) = WATCHED
                    .iter()
                    .find(|(name, _)| key.as_deref() == Some(*name))
                {
                    self.force.extend(keys.iter().copied());
                }
                self.schedule(DEBOUNCE);
            }
            Event::Timer => {
                self.deadline = None;
                self.reconcile();
            }
            Event::Poll => {
                if self.rescan() {
                    self.reconcile();
                }
            }
            Event::PoweredOn => self.schedule(AFTER_WAKE),
        }
    }

    /// Handles one request from another thread.
    fn command(&mut self, command: Command) {
        match command {
            Command::Desired(desired) => {
                if *desired != self.desired {
                    self.desired = *desired;
                    self.reconcile();
                }
            }
            Command::ReconcileAfter(delay) => self.schedule(delay),
            Command::RestoreAll(reply) => {
                // Off from now on: a later hot-plug or wake must not apply
                // again what this puts back.
                self.desired.active = false;
                let report = self.restore_all();
                let _ = reply.send(report);
            }
            Command::Stop => {
                STOPPING.with(|s| s.set(true));
                self.run_loop.stop();
            }
        }
    }

    /// Asks for a reconcile once things have been quiet for `delay`.
    fn schedule(&mut self, delay: Duration) {
        let deadline = next_deadline(
            self.deadline,
            CFAbsoluteTimeGetCurrent(),
            delay.as_secs_f64(),
        );
        self.deadline = Some(deadline);
        if let Some(timer) = &self.debounce {
            timer.set_next_fire_date(deadline);
        }
    }

    /// Brings the tracked services in line with what the client lists;
    /// whether the set changed.
    fn rescan(&mut self) -> bool {
        let Some(client) = &self.client else {
            return false;
        };
        let listed = client.services();
        let ids: BTreeSet<u64> = listed.iter().map(|(rid, _)| *rid).collect();
        let before = self.tracked.len();
        self.tracked.retain(|rid, _| ids.contains(rid));
        self.ignored.retain(|rid| ids.contains(rid));
        let mut changed = self.tracked.len() != before;
        for (rid, service) in listed {
            if self.tracked.contains_key(&rid) || self.ignored.contains(&rid) {
                continue;
            }
            let info = read_service(&service, rid);
            if !(info.is_keyboard || info.is_pointer) {
                self.ignored.insert(rid);
                continue;
            }
            if let (Some(register), true) = (client.spi.register_removal, client.hot_plug()) {
                // SAFETY: a valid service; the callback is a plain function
                // and the refcon carries the registry id, not a pointer.
                unsafe {
                    register(
                        CFRetained::as_ptr(&service).as_ptr(),
                        on_removed,
                        ptr::null_mut(),
                        ptr::without_provenance_mut(rid as usize),
                    );
                }
            }
            debug!(registry_id = rid, product = %info.product, "HID service appeared");
            self.tracked.insert(rid, Tracked { service, info });
            changed = true;
        }
        changed
    }

    /// Reads what every tracked service holds now.
    fn observe_all(&self) -> BTreeMap<u64, Observed> {
        let backend = self
            .client
            .as_ref()
            .map_or(HidBackend::Unavailable, HidClient::backend);
        self.tracked
            .iter()
            .map(|(rid, t)| (*rid, observe(&t.service, &t.info, backend)))
            .collect()
    }

    /// Makes every service carry what `desired` says, writing only
    /// differences, and publishes the result.
    fn reconcile(&mut self) {
        if self.client.is_none() {
            return;
        }
        self.rescan();
        let observed = self.observe_all();
        let services: Vec<HidService> = observed.values().map(|o| o.service.clone()).collect();
        let found = device::group_services(&services);
        let intents = devices::intents(&found, &observed, &self.journal.journal, &self.desired);
        let force = std::mem::take(&mut self.force);
        let retry = std::mem::take(&mut self.retry);
        let steps = devices::diff(&intents, &observed, &self.journal.journal, &force, &retry);
        let Some(client) = &self.client else {
            return;
        };
        let mut io = LiveIo {
            client,
            tracked: &self.tracked,
        };
        let report = devices::execute(steps, &mut io, &mut self.journal, journal::recover);
        self.retry.clone_from(&report.failed);
        let (applied, keyboards_mapped, mice_tuned) = devices::in_place(&intents, &report);
        let issues = devices::device_issues(&found, &observed);
        self.publish(HidSummary {
            backend: client.backend,
            services,
            devices: found,
            applied,
            keyboards_mapped,
            mice_tuned,
            issues,
            restore_pending: report.restore_failed,
        });
    }

    /// Puts back every journalled value now.
    fn restore_all(&mut self) -> ExecReport {
        // What failed to apply is no longer wanted once it is all put back.
        self.retry.clear();
        self.rescan();
        let observed = self.observe_all();
        let steps = devices::restore_everything(&observed, &self.journal.journal);
        let Some(client) = &self.client else {
            return ExecReport::default();
        };
        let mut io = LiveIo {
            client,
            tracked: &self.tracked,
        };
        let report = devices::execute(steps, &mut io, &mut self.journal, journal::recover);
        let mut summary = self.summary.clone();
        summary.applied.clear();
        summary.keyboards_mapped = 0;
        summary.mice_tuned = 0;
        summary.restore_pending = report.restore_failed;
        self.publish(summary);
        report
    }

    /// Tells the service about a changed summary; an unchanged one is not
    /// sent, so a reconcile with nothing to do causes no traffic.
    fn publish(&mut self, summary: HidSummary) {
        if summary != self.summary {
            self.summary = summary.clone();
            (self.on_summary)(summary);
        }
    }
}

/// How the HID thread starts.
pub(crate) struct HidStart {
    /// The journal as loaded (already emptied if it was another boot's).
    pub journal: JournalStore,
    /// Put back what the journal holds before applying anything: it is a
    /// previous run's, from this boot.
    pub recover: bool,
    /// What to make true first.
    pub desired: Desired,
    /// Called on the HID thread whenever the summary changes. Must not
    /// block.
    pub on_summary: Box<dyn Fn(HidSummary) + Send>,
}

/// The running HID thread.
pub(crate) struct HidThread {
    commands: Sender<Command>,
    /// The thread's run loop, woken after each command.
    run_loop: RunLoopHandle,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Disconnects when the thread has finished, for a bounded join.
    exited: Mutex<Option<Receiver<()>>>,
}

impl HidThread {
    /// Starts the thread, which recovers the journal when asked, applies
    /// `desired`, and then follows hot-plug, wake and commands. Returns
    /// with the first summary once it is ready (or after `wait`, with
    /// whatever is known). `None` when the thread could not be started.
    pub(crate) fn start(start: HidStart, wait: Duration) -> Option<(HidThread, HidSummary)> {
        let (commands, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<RunLoopHandle>(1);
        let (first_tx, first_rx) = mpsc::sync_channel::<HidSummary>(1);
        let (exit_tx, exit_rx) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("isthmus-hid".into())
            .spawn(move || {
                run(start, receiver, &ready_tx, &first_tx);
                drop(exit_tx);
            });
        let thread = match spawned {
            Ok(thread) => thread,
            Err(error) => {
                error!(?error, "could not start the HID thread");
                return None;
            }
        };
        // No timeout here: the handle is the thread's first act, and the
        // channel disconnects if the thread dies first. Giving up early
        // would leave a thread that applies settings and cannot be told to
        // put them back.
        let run_loop = ready_rx.recv().ok()?;
        let first = first_rx.recv_timeout(wait).unwrap_or_default();
        Some((
            HidThread {
                commands,
                run_loop,
                thread: Mutex::new(Some(thread)),
                exited: Mutex::new(Some(exit_rx)),
            },
            first,
        ))
    }

    /// Queues `command` and wakes the thread to handle it.
    fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            let block = RcBlock::new(|| post(Event::Commands));
            self.run_loop.perform(&block);
        }
    }

    /// Makes `desired` true; the thread reconciles at once if it changed.
    pub(crate) fn set_desired(&self, desired: Desired) {
        self.send(Command::Desired(Box::new(desired)));
    }

    /// Reconciles once things have been quiet for `delay` (after wake).
    pub(crate) fn reconcile_after(&self, delay: Duration) {
        self.send(Command::ReconcileAfter(delay));
    }

    /// Switches the HID side off and puts back everything, waiting at most
    /// `timeout`. `None` when the thread did not answer in time (what it
    /// could not restore stays journalled for the next start).
    pub(crate) fn restore_all(&self, timeout: Duration) -> Option<ExecReport> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.send(Command::RestoreAll(reply));
        answer.recv_timeout(timeout).ok()
    }

    /// Stops the run loop and joins the thread, waiting at most `timeout`.
    pub(crate) fn stop(&self, timeout: Duration) {
        self.send(Command::Stop);
        self.run_loop.stop();
        let exited = self.exited.lock().take();
        let finished = exited.is_none_or(|rx| {
            matches!(
                rx.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Disconnected)
            )
        });
        if finished {
            if let Some(thread) = self.thread.lock().take() {
                let _ = thread.join();
            }
        } else {
            warn!("the HID thread did not stop in time");
        }
    }
}

/// The HID thread: sets up the client, callbacks, timers and power
/// notifications on this thread's run loop, recovers and reconciles once,
/// then runs until stopped.
fn run(
    start: HidStart,
    commands: Receiver<Command>,
    ready: &SyncSender<RunLoopHandle>,
    first: &SyncSender<HidSummary>,
) {
    let (Some(run_loop), Some(handle)) = (CFRunLoop::current(), RunLoopHandle::current()) else {
        error!("the HID thread has no run loop");
        return;
    };
    if ready.send(handle).is_err() {
        return;
    }
    STOPPING.with(|s| s.set(false));
    let client = HidClient::open();
    if client.is_none() {
        warn!("the HID event system is unavailable");
    }
    // SAFETY: an extern static CF constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    let timer = |interval: f64, fire_in: f64, event: fn() -> Event| {
        let block = RcBlock::new(move |_timer: *mut CFRunLoopTimer| post(event()));
        // SAFETY: a default allocator and a block that only posts an event.
        let timer = unsafe {
            CFRunLoopTimer::with_handler(
                None,
                CFAbsoluteTimeGetCurrent() + fire_in,
                interval,
                0,
                0,
                Some(&*block),
            )
        };
        if let Some(timer) = &timer {
            run_loop.add_timer(Some(timer), mode);
        }
        timer
    };
    let debounce = timer(NEVER, NEVER, || Event::Timer);
    let poll = client.as_ref().and_then(|c| {
        let every = if c.hot_plug() { SAFETY_POLL } else { POLL }.as_secs_f64();
        timer(every, every, || Event::Poll)
    });
    let mut watched = Vec::new();
    if let Some(client) = client.as_ref().filter(|c| c.hot_plug()) {
        // SAFETY (whole block): a valid client and run loop; callbacks are
        // plain functions with NULL targets, so nothing they receive can
        // dangle; the key strings are kept alive in the state.
        unsafe {
            if let Some(register) = client.spi.register_matching {
                register(client.raw(), on_matched, ptr::null_mut(), ptr::null_mut());
            }
            if let Some(register) = client.spi.register_property {
                for (key, _) in WATCHED {
                    let key = cf::string(key);
                    register(
                        client.raw(),
                        CFRetained::as_ptr(&key).as_ptr(),
                        on_property,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                    watched.push(key);
                }
            }
            if let (Some(schedule), Some(mode)) = (client.spi.schedule, mode) {
                schedule(
                    client.raw(),
                    CFRetained::as_ptr(&run_loop).as_ptr(),
                    ptr::from_ref(mode),
                );
            }
        }
    }
    let power = Power::register(&run_loop);
    let state = HidState {
        client,
        run_loop: run_loop.clone(),
        commands,
        tracked: BTreeMap::new(),
        ignored: BTreeSet::new(),
        journal: start.journal,
        desired: start.desired,
        force: BTreeSet::new(),
        retry: BTreeSet::new(),
        debounce,
        deadline: None,
        _poll: poll,
        _power: power,
        _watched: watched,
        summary: HidSummary::default(),
        on_summary: start.on_summary,
    };
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
    let summary = STATE.with(|cell| {
        let mut guard = cell.borrow_mut();
        let state = guard.as_mut()?;
        if start.recover {
            let report = state.restore_all();
            info!(
                restored = report.restored,
                left = report.left,
                failed = report.restore_failed,
                "recovered the journal of a previous run"
            );
        }
        state.reconcile();
        Some(state.summary.clone())
    });
    let _ = first.send(summary.unwrap_or_default());
    post(Event::Commands);
    while !STOPPING.with(Cell::get) {
        let result = CFRunLoop::run_in_mode(mode, NEVER, false);
        // A run loop with nothing to wait on returns at once; never spin.
        if result == CFRunLoopRunResult::Finished {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().take() {
            if let Some(timer) = &state.debounce {
                timer.invalidate();
            }
            if let Some(timer) = &state._poll {
                timer.invalidate();
            }
            drop(state);
        }
    });
    EVENTS.with(|queue| queue.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Observing services only reads, and reads only the keys of the
    /// service's kind: keyboards their mapping and F-key mode, pointers
    /// their linear flag and acceleration. Run with `--nocapture` to see
    /// this Mac's current values and the system-wide ones.
    #[test]
    fn observing_reads_only_the_keys_of_each_services_kind() {
        let client = HidClient::open().expect("the HID event system answers a read-only client");
        for key in [
            "HIDMouseAcceleration",
            "HIDUseLinearScalingMouseAcceleration",
            "HIDFKeyMode",
        ] {
            println!("global {key} = {:?}", client.global(key));
        }
        for (rid, service) in client.services() {
            let info = read_service(&service, rid);
            let seen = observe(&service, &info, client.backend());
            println!(
                "{rid:#x} {:?}: values={:?} linear={} modifierKeys={}",
                info.product, seen.values, seen.publishes_linear, seen.system_modifier_keys
            );
            let pointer_keys = [PropKey::LinearScaling, PropKey::MouseAcceleration];
            let keyboard_keys = [PropKey::UserKeyMapping, PropKey::FKeyMode];
            if !info.is_pointer {
                assert!(pointer_keys.iter().all(|k| !seen.values.contains_key(k)));
                assert!(!seen.publishes_linear);
            }
            if !info.is_keyboard {
                assert!(keyboard_keys.iter().all(|k| !seen.values.contains_key(k)));
            }
        }
    }

    /// A Passive client that lists nothing is kept while the simple client
    /// sees no keyboard or pointer either (nothing attached at start, say
    /// behind a KVM switched away), and given up only when the simple client
    /// sees input the Passive one cannot.
    #[test]
    fn a_silent_passive_client_is_given_up_only_for_input_it_cannot_see() {
        assert!(keeps_passive(true, true));
        assert!(keeps_passive(true, false));
        assert!(keeps_passive(false, false));
        assert!(!keeps_passive(false, true));
    }

    /// A debounced reconcile waits for quiet, and a shorter request never
    /// pulls an already promised later deadline forward.
    #[test]
    fn debounce_deadlines_only_move_later() {
        assert_eq!(next_deadline(None, 100.0, 0.3), 100.3);
        assert_eq!(next_deadline(Some(101.5), 100.0, 0.3), 101.5);
        assert_eq!(next_deadline(Some(100.2), 100.1, 0.3), 100.1 + 0.3);
    }
}
