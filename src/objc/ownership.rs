//! ARC object ownership, explicit pools, zeroing weak storage and ivar cleanup.
use super::*;
use std::cell::Cell;

impl Session {
    pub(super) fn ownership_layout(
        &self,
        address: u64,
        start: usize,
        size: usize,
    ) -> Result<BTreeSet<usize>> {
        let mut slots = BTreeSet::new();
        if address == 0 {
            return Ok(slots);
        }
        let mut cursor = start;
        for index in 0..4096u64 {
            self.range(address + index, 1, false)?;
            let byte = unsafe { ((address + index) as *const u8).read() };
            if byte == 0 {
                return Ok(slots);
            }
            cursor = cursor
                .checked_add(usize::from(byte >> 4) * 8)
                .ok_or("Objective-C ownership layout overflow")?;
            for _ in 0..byte & 15 {
                if cursor < 8 || !cursor.is_multiple_of(8) || cursor + 8 > size {
                    return Err("Objective-C ownership layout outside instance".into());
                }
                slots.insert(cursor);
                cursor += 8;
            }
            if cursor > size {
                return Err("Objective-C ownership layout outside instance".into());
            }
        }
        Err("Objective-C ownership layout exceeds byte limit".into())
    }
    fn slot(&self, address: u64) {
        if address == 0 || !address.is_multiple_of(8) {
            fatal("unaligned or null Objective-C ownership slot");
        }
        let owned = self.allocations.iter().any(|(&base, a)| {
            address >= base
                && address
                    .checked_add(8)
                    .is_some_and(|end| end <= base + a.layout.size() as u64)
        });
        let stack = crate::platform::stack_contains(address)
            && crate::platform::stack_contains(address + 7);
        if !owned
            && !stack
            && !self.weak_slots.contains_key(&address)
            && self.range(address, 8, true).is_err()
        {
            fatal("Objective-C ownership slot outside writable guest storage");
        }
    }
    fn root_try_retain(&mut self, object: u64) -> bool {
        if object == 0 {
            return false;
        }
        if self.classes.contains_key(&object) {
            return true;
        }
        let allocation = self
            .allocations
            .get_mut(&object)
            .unwrap_or_else(|| fatal("retain requires runtime-owned instance"));
        if allocation.references == 0 {
            return false;
        }
        allocation.references = allocation
            .references
            .checked_add(1)
            .unwrap_or_else(|| fatal("Objective-C retain overflow"));
        true
    }
    fn rr_method(&mut self, object: u64, name: &[u8]) -> Option<(u64, u64)> {
        if object == 0 || self.classes.contains_key(&object) {
            return None;
        }
        let class = self.object_class(object).unwrap_or_else(|e| fatal(e));
        let imp = self.lookup(class, name)?;
        Some((imp, self.intern(name.to_vec())))
    }
    fn weak_remove(&mut self, slot: u64) {
        if let Some(old) = self.weak_slots.remove(&slot)
            && let Some(slots) = self.weak_targets.get_mut(&old)
        {
            slots.remove(&slot);
            if slots.is_empty() {
                self.weak_targets.remove(&old);
            }
        }
    }
    fn weak_assign(&mut self, slot: u64, object: u64) {
        self.slot(slot);
        if self.weak_slots.len() >= 65536 && !self.weak_slots.contains_key(&slot) {
            fatal("Objective-C weak slot limit exceeded");
        }
        self.weak_remove(slot);
        self.weak_slots.insert(slot, object);
        if object != 0 {
            self.weak_targets.entry(object).or_default().insert(slot);
        }
        unsafe {
            (slot as *mut u64).write(object);
        }
    }
    fn weak_clear(&mut self, object: u64) {
        if let Some(slots) = self.weak_targets.remove(&object) {
            for slot in slots {
                if unsafe { (slot as *const u64).read() } != object {
                    fatal("Objective-C weak storage changed without storeWeak");
                }
                unsafe {
                    (slot as *mut u64).write(0);
                }
                self.weak_slots.insert(slot, 0);
            }
        }
    }
}

unsafe extern "sysv64" fn root_retain(object: u64) -> u64 {
    if object == 0 {
        return 0;
    }
    let mut lock = registry().0.lock().unwrap();
    if !lock.as_mut().unwrap().root_try_retain(object) {
        fatal("retain of deallocating Objective-C object");
    }
    object
}
unsafe extern "sysv64" fn root_try_retain(object: u64) -> i8 {
    i8::from(
        registry()
            .0
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .root_try_retain(object),
    )
}
unsafe extern "sysv64" fn root_is_deallocating(object: u64) -> i8 {
    if object == 0 {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    if s.classes.contains_key(&object) {
        return 0;
    }
    i8::from(
        s.allocations
            .get(&object)
            .unwrap_or_else(|| fatal("unknown Objective-C instance"))
            .references
            == 0,
    )
}
unsafe extern "sysv64" fn root_retain_count(object: u64) -> usize {
    if object == 0 {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    if s.classes.contains_key(&object) {
        return usize::MAX;
    }
    s.allocations
        .get(&object)
        .unwrap_or_else(|| fatal("unknown Objective-C instance"))
        .references
}
unsafe fn invoke(object: u64, method: (u64, u64)) -> u64 {
    let f: unsafe extern "sysv64" fn(u64, u64) -> u64 =
        unsafe { std::mem::transmute(method.0 as usize) };
    unsafe { f(object, method.1) }
}
unsafe fn invoke_bool(object: u64, method: (u64, u64)) -> bool {
    let f: unsafe extern "sysv64" fn(u64, u64) -> i8 =
        unsafe { std::mem::transmute(method.0 as usize) };
    unsafe { f(object, method.1) != 0 }
}
pub(super) unsafe extern "sysv64" fn retain(object: u64) -> u64 {
    let method = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .rr_method(object, b"retain");
    if let Some(method) = method {
        unsafe { invoke(object, method) }
    } else {
        unsafe { root_retain(object) }
    }
}
pub(super) unsafe extern "sysv64" fn release(object: u64) {
    let method = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .rr_method(object, b"release");
    if let Some(method) = method {
        unsafe {
            call_void(method.0, object, method.1);
        }
    } else {
        unsafe {
            root_release(object);
        }
    }
}
pub(super) unsafe extern "sysv64" fn autorelease(object: u64) -> u64 {
    let method = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .rr_method(object, b"autorelease");
    if let Some(method) = method {
        unsafe { invoke(object, method) }
    } else {
        unsafe { autorelease_raw(object) }
    }
}
unsafe extern "sysv64" fn root_release(object: u64) {
    if object == 0 {
        return;
    }
    let (action, generation) = {
        let mut lock = registry().0.lock().unwrap();
        let s = lock.as_mut().unwrap();
        if s.classes.contains_key(&object) {
            return;
        }
        let allocation = s
            .allocations
            .get_mut(&object)
            .unwrap_or_else(|| fatal("release requires runtime-owned instance"));
        if allocation.references == 0 {
            fatal("recursive Objective-C deallocation");
        }
        allocation.references -= 1;
        if allocation.references != 0 {
            return;
        }
        let generation = allocation.generation;
        (s.rr_method(object, b"dealloc"), generation)
    };
    if let Some(method) = action {
        unsafe {
            call_void(method.0, object, method.1);
        }
    }
    let still_owned = registry()
        .0
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .allocations
        .get(&object)
        .is_some_and(|a| a.generation == generation);
    if still_owned {
        unsafe {
            dispose(object);
        }
    }
}
unsafe extern "sysv64" fn destruct_instance(object: u64) -> u64 {
    if object == 0 {
        return 0;
    }
    let calls = {
        let mut lock = registry().0.lock().unwrap();
        let s = lock.as_mut().unwrap();
        let allocation = s
            .allocations
            .get_mut(&object)
            .unwrap_or_else(|| fatal("destructInstance requires runtime-owned instance"));
        if allocation.destructed {
            return object;
        }
        allocation.destructed = true;
        let mut class = s.object_class(object).unwrap_or_else(|e| fatal(e));
        let selector = s.intern(b".cxx_destruct".to_vec());
        let mut calls = Vec::new();
        while class != 0 {
            let data = &s.classes[&class];
            if data.destructor {
                calls.push((data.methods[b".cxx_destruct".as_slice()], selector));
            }
            class = data.superclass;
        }
        calls
    };
    for method in calls {
        unsafe {
            call_void(method.0, object, method.1);
        }
    }
    registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .weak_clear(object);
    object
}
pub(super) unsafe fn dispose(object: u64) -> u64 {
    if object == 0 {
        return 0;
    }
    {
        let mut lock = registry().0.lock().unwrap();
        lock.as_mut()
            .unwrap()
            .allocations
            .get_mut(&object)
            .unwrap_or_else(|| fatal("object_dispose requires runtime-owned instance"))
            .references = 0;
    }
    unsafe {
        destruct_instance(object);
    }
    let allocation = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .allocations
        .remove(&object)
        .unwrap_or_else(|| fatal("recursive object_dispose"));
    unsafe {
        dealloc(object as *mut u8, allocation.layout);
    }
    0
}
unsafe extern "sysv64" fn store_strong(slot: u64, object: u64) {
    let previous = {
        let lock = registry().0.lock().unwrap();
        lock.as_ref().unwrap().slot(slot);
        unsafe { (slot as *const u64).read() }
    };
    if previous == object {
        return;
    }
    unsafe {
        retain(object);
        (slot as *mut u64).write(object);
        release(previous);
    }
}
unsafe extern "sysv64" fn set_property_nonatomic(
    object: u64,
    _selector: u64,
    value: u64,
    offset: i64,
) {
    let slot = object
        .checked_add_signed(offset)
        .unwrap_or_else(|| fatal("Objective-C property offset overflow"));
    unsafe {
        store_strong(slot, value);
    }
}

unsafe fn store_weak(slot: u64, object: u64, initialize: bool, or_nil: bool) -> u64 {
    let (pinned, method, class) = {
        let mut lock = registry().0.lock().unwrap();
        let s = lock.as_mut().unwrap();
        s.slot(slot);
        if initialize && s.weak_slots.contains_key(&slot) {
            fatal("objc_initWeak on initialized storage");
        }
        let pinned = s.root_try_retain(object);
        let method = if pinned {
            s.rr_method(object, b"allowsWeakReference")
        } else {
            None
        };
        let class = if pinned && !s.classes.contains_key(&object) {
            s.object_class(object).unwrap_or_else(|e| fatal(e))
        } else {
            0
        };
        (pinned, method, class)
    };
    if class != 0 {
        initialize_class(class);
    }
    let allowed = pinned && method.is_none_or(|m| unsafe { invoke_bool(object, m) });
    if !allowed && object != 0 && !or_nil {
        if pinned {
            unsafe {
                root_release(object);
            }
        }
        fatal("cannot form weak reference to deallocating or weak-unavailable object");
    }
    let result = if allowed { object } else { 0 };
    registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .weak_assign(slot, result);
    if pinned {
        unsafe {
            root_release(object);
        }
    }
    result
}
unsafe extern "sysv64" fn init_weak(slot: u64, object: u64) -> u64 {
    unsafe { store_weak(slot, object, true, false) }
}
unsafe extern "sysv64" fn init_weak_or_nil(slot: u64, object: u64) -> u64 {
    unsafe { store_weak(slot, object, true, true) }
}
unsafe extern "sysv64" fn assign_weak(slot: u64, object: u64) -> u64 {
    unsafe { store_weak(slot, object, false, false) }
}
unsafe extern "sysv64" fn assign_weak_or_nil(slot: u64, object: u64) -> u64 {
    unsafe { store_weak(slot, object, false, true) }
}
unsafe extern "sysv64" fn destroy_weak(slot: u64) {
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    s.slot(slot);
    s.weak_remove(slot);
}
unsafe extern "sysv64" fn load_weak_retained(slot: u64) -> u64 {
    let (object, method) = {
        let mut lock = registry().0.lock().unwrap();
        let s = lock.as_mut().unwrap();
        s.slot(slot);
        let object = unsafe { (slot as *const u64).read() };
        if object == 0 {
            return 0;
        }
        if s.weak_slots.get(&slot) != Some(&object) {
            fatal("loadWeak requires registered weak storage");
        }
        if !s.root_try_retain(object) {
            return 0;
        }
        (object, s.rr_method(object, b"retainWeakReference"))
    };
    if let Some(method) = method {
        // Pin under the registry lock, then run the custom weak retain outside
        // it. A successful callback supplies the returned +1; release the pin.
        let retained = unsafe { invoke_bool(object, method) };
        unsafe {
            root_release(object);
        }
        if !retained {
            return 0;
        }
    }
    object
}
unsafe extern "sysv64" fn load_weak(slot: u64) -> u64 {
    unsafe { autorelease(load_weak_retained(slot)) }
}
unsafe extern "sysv64" fn copy_weak(destination: u64, source: u64) {
    let object = unsafe { load_weak_retained(source) };
    unsafe {
        init_weak(destination, object);
        release(object);
    }
}
unsafe extern "sysv64" fn move_weak(destination: u64, source: u64) {
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    s.slot(source);
    s.slot(destination);
    if destination == source || s.weak_slots.contains_key(&destination) {
        fatal("moveWeak requires uninitialized destination");
    }
    let object = unsafe { (source as *const u64).read() };
    if object != 0 && s.weak_slots.get(&source) != Some(&object) {
        fatal("moveWeak requires registered source");
    }
    s.weak_assign(destination, object);
    s.weak_assign(source, 0);
}

#[derive(Clone, Copy)]
struct ReturnValue {
    object: u64,
    claim_return: u64,
}
thread_local! { static RETURN: Cell<Option<ReturnValue>> = const { Cell::new(None) }; }
pub(super) fn clear_return() {
    RETURN.with(|r| r.set(None));
}
pub(super) fn flush_return() {
    if let Some(value) = RETURN.with(|r| r.take()) {
        unsafe {
            autorelease(value.object);
        }
    }
}
// Inspect only admitted, mapped x86 instructions and binding slots. Recognize
// mov rax,rdi + call rel32 / call [rip+disp32], optionally through a Mach stub.
fn claim_site(pc: u64) -> Option<u64> {
    let lock = registry().0.lock().ok()?;
    let s = lock.as_ref()?;
    let bytes = |address: u64, size: usize| -> Option<&[u8]> {
        s.range(address, size as u64, false).ok()?;
        Some(unsafe { std::slice::from_raw_parts(address as *const u8, size) })
    };
    if bytes(pc, 3)? != [0x48, 0x89, 0xc7] {
        return None;
    }
    let call = pc.checked_add(3)?;
    let code = bytes(call, 6)?;
    let (mut target, end) = if code[0] == 0xe8 {
        let end = call + 5;
        (
            end.checked_add_signed(i64::from(i32::from_le_bytes(code[1..5].try_into().ok()?)))?,
            end,
        )
    } else if code[..2] == [0xff, 0x15] {
        let end = call + 6;
        let slot =
            end.checked_add_signed(i64::from(i32::from_le_bytes(code[2..6].try_into().ok()?)))?;
        (s.u64(slot).ok()?, end)
    } else {
        return None;
    };
    if let Some(stub) = bytes(target, 6)
        && stub[..2] == [0xff, 0x25]
    {
        let slot = (target + 6)
            .checked_add_signed(i64::from(i32::from_le_bytes(stub[2..6].try_into().ok()?)))?;
        target = s.u64(slot).ok()?;
    }
    if [
        adapter_objc_claim_return as *const () as u64,
        adapter_objc_unsafe_claim_return as *const () as u64,
    ]
    .contains(&target)
    {
        Some(end)
    } else {
        None
    }
}
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn adapter_objc_offer_return(object: u64, pc: u64) -> u64 {
    flush_return();
    if object != 0
        && let Some(claim_return) = claim_site(pc)
    {
        RETURN.with(|r| {
            r.set(Some(ReturnValue {
                object,
                claim_return,
            }))
        });
        return object;
    }
    unsafe { autorelease(object) }
}
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn adapter_objc_accept_return(
    object: u64,
    pc: u64,
    unsafe_claim: u64,
) -> u64 {
    let pending = RETURN.with(|r| r.take());
    if let Some(value) = pending {
        if value.object == object && value.claim_return == pc {
            if unsafe_claim != 0 {
                unsafe {
                    release(object);
                }
            }
            return object;
        }
        unsafe {
            autorelease(value.object);
        }
    }
    if unsafe_claim == 0 {
        unsafe { retain(object) }
    } else {
        object
    }
}
unsafe extern "sysv64" fn retain_autorelease(object: u64) -> u64 {
    unsafe { autorelease(retain(object)) }
}
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn adapter_objc_retain_offer_return(object: u64, pc: u64) -> u64 {
    unsafe {
        retain(object);
        adapter_objc_offer_return(object, pc)
    }
}
unsafe extern "sysv64" {
    fn adapter_objc_autorelease_return();
    fn adapter_objc_retain_autorelease_return();
    fn adapter_objc_claim_return();
    fn adapter_objc_unsafe_claim_return();
}
pub(super) fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "__objc_rootRetain" => root_retain as *const () as u64,
        "__objc_rootRelease" => root_release as *const () as u64,
        "__objc_rootAutorelease" => autorelease_raw as *const () as u64,
        "__objc_rootRetainCount" => root_retain_count as *const () as u64,
        "__objc_rootTryRetain" => root_try_retain as *const () as u64,
        "__objc_rootIsDeallocating" => root_is_deallocating as *const () as u64,
        "_objc_destructInstance" => destruct_instance as *const () as u64,
        "_objc_storeStrong" => store_strong as *const () as u64,
        "_objc_setProperty_nonatomic" => set_property_nonatomic as *const () as u64,
        "_objc_initWeak" => init_weak as *const () as u64,
        "_objc_initWeakOrNil" => init_weak_or_nil as *const () as u64,
        "_objc_storeWeak" => assign_weak as *const () as u64,
        "_objc_storeWeakOrNil" => assign_weak_or_nil as *const () as u64,
        "_objc_destroyWeak" => destroy_weak as *const () as u64,
        "_objc_loadWeak" => load_weak as *const () as u64,
        "_objc_loadWeakRetained" => load_weak_retained as *const () as u64,
        "_objc_copyWeak" => copy_weak as *const () as u64,
        "_objc_moveWeak" => move_weak as *const () as u64,
        "_objc_autoreleaseReturnValue" => adapter_objc_autorelease_return as *const () as u64,
        "_objc_retainAutoreleaseReturnValue" => {
            adapter_objc_retain_autorelease_return as *const () as u64
        }
        "_objc_retainAutoreleasedReturnValue" | "_objc_claimAutoreleasedReturnValue" => {
            adapter_objc_claim_return as *const () as u64
        }
        "_objc_unsafeClaimAutoreleasedReturnValue" => {
            adapter_objc_unsafe_claim_return as *const () as u64
        }
        "_objc_retainAutorelease" => retain_autorelease as *const () as u64,
        _ => return None,
    })
}
