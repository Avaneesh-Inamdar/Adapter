//! Bounded Intel Objective-C ABI support. Metadata remains Apple-layout data;
//! runtime bookkeeping is owned separately and never changes class data bits.
use crate::Result;
use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ffi::{CString, c_char},
    sync::{Condvar, Mutex, OnceLock},
    thread::ThreadId,
};

#[derive(Clone)]
pub struct Region {
    pub address: u64,
    pub size: u64,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}
pub struct MetadataSection {
    pub name: String,
    pub address: u64,
    pub size: u64,
}
pub struct Image {
    pub regions: Vec<Region>,
    pub sections: Vec<MetadataSection>,
}
#[derive(Clone)]
struct Class {
    isa: u64,
    superclass: u64,
    name: Vec<u8>,
    size: usize,
    meta: bool,
    methods: BTreeMap<Vec<u8>, u64>,
}
struct Allocation {
    layout: Layout,
    references: usize,
}
struct Session {
    regions: Vec<Region>,
    classes: BTreeMap<u64, Class>,
    names: BTreeMap<Vec<u8>, u64>,
    selectors: BTreeMap<Vec<u8>, CString>,
    allocations: BTreeMap<u64, Allocation>,
    initializing: BTreeMap<u64, Option<ThreadId>>,
    /// Category +load implementations in discovery order; Apple calls a class's
    /// own +load first and then each category +load, so they stay separate.
    category_loads: Vec<(u64, u64)>,
    loaded: bool,
}
fn registry() -> &'static (Mutex<Option<Session>>, Condvar) {
    static R: OnceLock<(Mutex<Option<Session>>, Condvar)> = OnceLock::new();
    R.get_or_init(|| (Mutex::new(None), Condvar::new()))
}
fn fatal(message: impl std::fmt::Display) -> ! {
    eprintln!("adapter-run [objc]: {message}");
    std::process::exit(125)
}
impl Session {
    fn range(&self, address: u64, size: u64, write: bool) -> Result<()> {
        let end = address
            .checked_add(size)
            .ok_or("Objective-C range overflow")?;
        if self.regions.iter().any(|r| {
            r.readable
                && (!write || r.writable)
                && address >= r.address
                && end <= r.address.saturating_add(r.size)
        }) {
            Ok(())
        } else {
            Err("Objective-C metadata pointer outside permitted mapped segment".into())
        }
    }
    fn u64(&self, address: u64) -> Result<u64> {
        self.range(address, 8, false)?;
        Ok(unsafe { (address as *const u64).read_unaligned() })
    }
    fn u32(&self, address: u64) -> Result<u32> {
        self.range(address, 4, false)?;
        Ok(unsafe { (address as *const u32).read_unaligned() })
    }
    fn string(&self, address: u64) -> Result<Vec<u8>> {
        if let Some((key, _)) = self
            .selectors
            .iter()
            .find(|(_, v)| v.as_ptr() as u64 == address)
        {
            return Ok(key.clone());
        }
        // Bound every byte before dereferencing. C strings may end at a page edge.
        let mut bytes = Vec::new();
        for offset in 0..4096 {
            self.range(
                address
                    .checked_add(offset)
                    .ok_or("Objective-C string overflow")?,
                1,
                false,
            )?;
            let byte = unsafe { ((address + offset) as *const u8).read() };
            if byte == 0 {
                return Ok(bytes);
            }
            bytes.push(byte);
        }
        Err("Objective-C string exceeds 4096-byte limit".into())
    }
    fn intern(&mut self, name: Vec<u8>) -> u64 {
        self.selectors
            .entry(name.clone())
            .or_insert_with(|| CString::new(name).unwrap())
            .as_ptr() as u64
    }
    fn read_class(&mut self, address: u64) -> Result<()> {
        self.read_class_at(address, 0)
    }
    fn read_class_at(&mut self, address: u64, depth: usize) -> Result<()> {
        if self.classes.contains_key(&address) {
            return Ok(());
        }
        if depth > 128 {
            return Err("Objective-C class graph exceeds 128 levels".into());
        }
        if self.classes.len() >= 8192 {
            return Err("Objective-C class limit exceeded".into());
        }
        self.range(address, 40, false)?;
        let isa = self.u64(address)?;
        let superclass = self.u64(address + 8)?;
        let bits = self.u64(address + 32)?;
        if bits & 7 != 0 {
            return Err("Swift or flagged Objective-C class data unsupported".into());
        }
        let ro = bits;
        self.range(ro, 72, false)?;
        let flags = self.u32(ro)?;
        if flags & !(1 | 2 | 16) != 0 {
            return Err(format!(
                "Objective-C class flags {flags:#x} unsupported (ARC/C++/weak/Swift)"
            ));
        }
        let size = self.u32(ro + 8)? as usize;
        let start = self.u32(ro + 4)? as usize;
        if size > 1024 * 1024 || start > size {
            return Err("Objective-C instance layout invalid".into());
        }
        for offset in [16, 40, 56, 64] {
            if self.u64(ro + offset)? != 0 {
                return Err(
                    "Objective-C ownership layouts, protocols and properties unsupported".into(),
                );
            }
        }
        let name = self.string(self.u64(ro + 24)?)?;
        if name.is_empty() {
            return Err("Objective-C class has empty name".into());
        }
        let mut methods = BTreeMap::new();
        let list = self.u64(ro + 32)?;
        if list != 0 {
            methods = self.read_methods(list)?;
        }
        let ivars = self.u64(ro + 48)?;
        if ivars != 0 {
            if self.u32(ivars)? != 32 {
                return Err("Objective-C ivar entry size unsupported".into());
            }
            let count = self.u32(ivars + 4)? as u64;
            if count > 65536 {
                return Err("Objective-C ivar count exceeds limit".into());
            }
            self.range(ivars + 8, count * 32, false)?;
            for index in 0..count {
                let ivar = ivars + 8 + index * 32;
                let offset = self.u32(self.u64(ivar)?)? as usize;
                let field_size = self.u32(ivar + 28)? as usize;
                self.string(self.u64(ivar + 8)?)?;
                let types = self.string(self.u64(ivar + 16)?)?;
                if types.first() == Some(&b'@')
                    || offset.checked_add(field_size).is_none_or(|n| n > size)
                {
                    return Err(
                        "Objective-C object ivar ownership or invalid layout unsupported".into(),
                    );
                }
            }
        }
        self.classes.insert(
            address,
            Class {
                isa,
                superclass,
                name,
                size,
                meta: flags & 1 != 0,
                methods,
            },
        );
        self.read_class_at(isa, depth + 1)?;
        if superclass != 0 {
            self.read_class_at(superclass, depth + 1)?;
        }
        Ok(())
    }
    fn targeted(base: u64, offset: i32) -> Result<u64> {
        (base as i64)
            .checked_add(i64::from(offset))
            .map(|value| value as u64)
            .ok_or_else(|| "Objective-C relative offset overflow".into())
    }
    /// Validate one method list. Clang emits 24-byte pointer entries unless the
    /// target uses relative method lists, where the header's 0x80000000 small
    /// flag selects 12-byte entries with three field-relative 32-bit offsets.
    /// Outside the dyld shared cache a relative selector field names a
    /// `__objc_selrefs` slot, so the name pointer is read indirectly.
    fn read_methods(&mut self, list: u64) -> Result<BTreeMap<Vec<u8>, u64>> {
        let mut methods = BTreeMap::new();
        if list == 0 {
            return Ok(methods);
        }
        let header = self.u32(list)?;
        let relative = header & 0x8000_0000 != 0;
        if header & 0x4000_0000 != 0 {
            return Err("Objective-C direct-selector method lists unsupported".into());
        }
        let stride = header & !0xffff_0003;
        let stride = if relative {
            if stride != 12 {
                return Err("Objective-C relative method entry size unsupported".into());
            }
            12u64
        } else {
            if stride != 24 {
                return Err("Objective-C absolute method entry size unsupported".into());
            }
            24u64
        };
        let count = self.u32(list + 4)? as u64;
        if count > 65536 {
            return Err("Objective-C method limit exceeded".into());
        }
        self.range(list + 8, count * stride, false)?;
        for index in 0..count {
            let item = list + 8 + index * stride;
            let (selector, types_address, imp) = if relative {
                let slot = Self::targeted(item, self.u32(item)? as i32)?;
                let types = Self::targeted(item + 4, self.u32(item + 4)? as i32)?;
                (
                    self.string(self.u64(slot)?)?,
                    types,
                    Self::targeted(item + 8, self.u32(item + 8)? as i32)?,
                )
            } else {
                (
                    self.string(self.u64(item)?)?,
                    self.u64(item + 8)?,
                    self.u64(item + 16)?,
                )
            };
            let types = self.string(types_address)?;
            if types.contains(&b'!') {
                return Err("Objective-C vector calling convention unsupported".into());
            }
            let result = types.iter().copied().find(|c| !b"rnNoORV".contains(c));
            if result.is_none() || matches!(result, Some(b'{' | b'(' | b'[' | b'D' | b'j' | b'!')) {
                return Err(
                    "Objective-C aggregate, long-double or vector return unsupported".into(),
                );
            }
            if !self
                .regions
                .iter()
                .any(|r| r.executable && imp >= r.address && imp < r.address.saturating_add(r.size))
            {
                return Err("Objective-C method IMP outside executable mapped segment".into());
            }
            if selector.is_empty() || methods.insert(selector.clone(), imp).is_some() {
                return Err("Objective-C empty or duplicate method selector".into());
            }
            self.intern(selector);
        }
        Ok(methods)
    }
    fn lookup(&self, mut class: u64, selector: &[u8]) -> Option<u64> {
        for _ in 0..=self.classes.len() {
            let data = self.classes.get(&class)?;
            if let Some(imp) = data.methods.get(selector) {
                return Some(*imp);
            }
            class = data.superclass;
        }
        None
    }
    fn object_class(&self, object: u64) -> Result<u64> {
        if object & 1 != 0 {
            return Err("Objective-C tagged pointer unsupported".into());
        }
        if let Some(class) = self.classes.get(&object) {
            return Ok(class.isa);
        }
        if !self.allocations.contains_key(&object) {
            self.range(object, 8, false)?;
        }
        let class = unsafe { (object as *const u64).read_unaligned() };
        if !self.classes.contains_key(&class) {
            return Err("unknown/nonpointer/tagged Objective-C isa".into());
        }
        Ok(class)
    }
}

pub struct Guard;
pub fn begin(images: Vec<Image>) -> Result<Guard> {
    let mut lock = registry()
        .0
        .lock()
        .map_err(|_| "Objective-C registry poisoned")?;
    if lock.is_some() {
        return Err("concurrent Objective-C session unsupported".into());
    }
    let mut session = Session {
        regions: images.iter().flat_map(|i| i.regions.clone()).collect(),
        classes: BTreeMap::new(),
        names: BTreeMap::new(),
        selectors: BTreeMap::new(),
        allocations: BTreeMap::new(),
        initializing: BTreeMap::new(),
        category_loads: Vec::new(),
        loaded: false,
    };
    let mut patches = Vec::new();
    let mut references = Vec::new();
    let mut categories = Vec::new();
    for image in &images {
        for section in &image.sections {
            match section.name.as_str() {
                "__objc_imageinfo" => {
                    if section.size != 8 || session.u32(section.address)? != 0 {
                        return Err("Objective-C image info version/size unsupported".into());
                    }
                    let flags = session.u32(section.address + 4)?;
                    // Bit 6 means category class properties are present in the ABI;
                    // it is emitted even for images without any categories.
                    if flags & !64 != 0 {
                        return Err(format!("Objective-C image flags {flags:#x} unsupported"));
                    }
                }
                "__objc_classlist" | "__objc_nlclslist" | "__objc_selrefs" | "__objc_classrefs"
                | "__objc_superrefs" | "__objc_catlist" | "__objc_nlcatlist" => {
                    if section.size % 8 != 0 || section.size / 8 > 8192 {
                        return Err("Objective-C pointer section invalid size".into());
                    }
                    session.range(section.address, section.size, false)?;
                    for offset in (0..section.size).step_by(8) {
                        let slot = section.address + offset;
                        let value = session.u64(slot)?;
                        match section.name.as_str() {
                            "__objc_classlist" | "__objc_nlclslist" => {
                                session.read_class(value)?;
                                if session.classes[&value].meta {
                                    return Err(
                                        "Objective-C class list contains a metaclass".into()
                                    );
                                }
                            }
                            "__objc_selrefs" => {
                                session.range(slot, 8, true)?;
                                let name = session.string(value)?;
                                let canonical = session.intern(name);
                                patches.push((slot, canonical));
                            }
                            "__objc_catlist" | "__objc_nlcatlist" => {
                                // A category carrying +load is emitted in
                                // __objc_nlcatlist; the linker here also leaves
                                // the same record in __objc_catlist, and
                                // attaching it twice would double its methods.
                                if !categories.contains(&value) {
                                    categories.push(value);
                                }
                            }
                            _ => references.push(value),
                        }
                    }
                }
                "__objc_const" | "__objc_data" | "__objc_ivar" | "__objc_methname"
                | "__objc_methtype" | "__objc_classname" | "__objc_methlist" => {}
                name if name.starts_with("__objc_") => {
                    return Err(format!("Objective-C section {name} unsupported"));
                }
                _ => {}
            }
        }
    }
    for (&address, class) in &session.classes {
        if !class.meta {
            if session.names.insert(class.name.clone(), address).is_some() {
                return Err("duplicate Objective-C class name".into());
            }
            let mut seen = BTreeSet::new();
            let mut cursor = address;
            while cursor != 0 {
                if !seen.insert(cursor) {
                    return Err("Objective-C superclass cycle".into());
                }
                let item = session
                    .classes
                    .get(&cursor)
                    .ok_or("missing Objective-C superclass")?;
                if item.meta {
                    return Err("Objective-C instance class inherits metaclass".into());
                }
                cursor = item.superclass;
            }
            let meta = &session.classes[&class.isa];
            let expected_meta_super = if class.superclass == 0 {
                address
            } else {
                session.classes[&class.superclass].isa
            };
            if !meta.meta || meta.name != class.name || meta.superclass != expected_meta_super {
                return Err("Objective-C class/metaclass relationship invalid".into());
            }
            let mut root = address;
            while session.classes[&root].superclass != 0 {
                root = session.classes[&root].superclass;
            }
            if meta.isa != session.classes[&root].isa {
                return Err("Objective-C metaclass isa does not point to root metaclass".into());
            }
            if class.superclass != 0 && class.size < session.classes[&class.superclass].size {
                return Err("Objective-C subclass instance smaller than superclass".into());
            }
        }
    }
    for value in references {
        if !session.classes.contains_key(&value) {
            return Err("Objective-C class/super reference is not registered".into());
        }
    }
    // Attach categories after every class in the graph is registered, so a
    // category may target a class declared later in the same image or in a
    // dependency. Apple gives category methods priority over the class's own
    // methods; one selector supplied by two categories of the same class is
    // ambiguous and rejected rather than resolved by link order.
    let mut claimed: BTreeMap<(u64, bool), BTreeSet<Vec<u8>>> = BTreeMap::new();
    for category in categories {
        session.range(category, 48, false)?;
        let name = session.string(session.u64(category)?)?;
        let class = session.u64(category + 8)?;
        let meta = match session.classes.get(&class) {
            Some(data) if !data.meta => data.isa,
            Some(_) => return Err("Objective-C category names a metaclass".into()),
            None => {
                return Err(format!(
                    "Objective-C category {} names an unregistered class",
                    String::from_utf8_lossy(&name)
                ));
            }
        };
        if name.is_empty() {
            return Err("Objective-C category has an empty name".into());
        }
        if session.u64(category + 32)? != 0 || session.u64(category + 40)? != 0 {
            return Err("Objective-C category protocols and properties unsupported".into());
        }
        let instance = session.read_methods(session.u64(category + 16)?)?;
        let class_methods = session.read_methods(session.u64(category + 24)?)?;
        for (is_instance, target, methods) in
            [(true, class, instance), (false, meta, class_methods)]
        {
            for (selector, imp) in methods {
                // +load is additive: Apple calls the class's own +load and then
                // each category +load, so it must not replace the class entry.
                if !is_instance && selector.as_slice() == b"load" {
                    session.category_loads.push((class, imp));
                    continue;
                }
                if !claimed
                    .entry((class, is_instance))
                    .or_default()
                    .insert(selector.clone())
                {
                    return Err(format!(
                        "Objective-C selector {} is defined by two categories of {}",
                        String::from_utf8_lossy(&selector),
                        String::from_utf8_lossy(&name)
                    ));
                }
                session
                    .classes
                    .get_mut(&target)
                    .ok_or("Objective-C category target class missing")?
                    .methods
                    .insert(selector, imp);
            }
        }
    }
    // Do not partially mutate guest metadata if any validation above fails.
    for (slot, value) in patches {
        unsafe {
            (slot as *mut u64).write_unaligned(value);
        }
    }
    *lock = Some(session);
    Ok(Guard)
}
impl Guard {
    /// Call validated, own +load implementations in superclass-first order.
    /// The loader invokes this after installing all guest runtime guards.
    pub fn initialize(&self) -> Result<()> {
        let calls = {
            let mut lock = registry()
                .0
                .lock()
                .map_err(|_| "Objective-C registry poisoned")?;
            let s = lock.as_mut().ok_or("Objective-C session absent")?;
            if s.loaded {
                return Ok(());
            }
            s.loaded = true;
            let selector = s.intern(b"load".to_vec());
            let mut classes: Vec<_> = s
                .classes
                .iter()
                .filter(|(_, c)| !c.meta)
                .map(|(p, _)| *p)
                .collect();
            classes.sort_by_key(|p| {
                let mut depth = 0;
                let mut cursor = *p;
                while let Some(c) = s.classes.get(&cursor) {
                    depth += 1;
                    cursor = c.superclass;
                }
                depth
            });
            // Apple calls every class +load (superclass first) and only then
            // each category +load, so the two passes stay separate.
            let mut calls = Vec::new();
            for class in classes {
                if let Some(imp) = s.classes[&s.classes[&class].isa]
                    .methods
                    .get(b"load".as_slice())
                {
                    calls.push((*imp, class, selector));
                }
            }
            for (owner, imp) in &s.category_loads {
                calls.push((*imp, *owner, selector));
            }
            calls
        };
        for (imp, class, selector) in calls {
            unsafe {
                call_void(imp, class, selector);
            }
        }
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        POOLS.with(|p| p.borrow_mut().clear());
        if let Ok(mut lock) = registry().0.lock()
            && let Some(session) = lock.take()
        {
            for (pointer, allocation) in session.allocations {
                unsafe {
                    dealloc(pointer as *mut u8, allocation.layout);
                }
            }
        }
    }
}
unsafe fn call_void(imp: u64, object: u64, selector: u64) {
    let f: unsafe extern "sysv64" fn(u64, u64) = unsafe { std::mem::transmute(imp as usize) };
    unsafe {
        f(object, selector);
    }
}
fn initialize_class(class: u64) {
    let (mutex, wake) = registry();
    let thread = std::thread::current().id();
    let superclass = {
        let lock = mutex.lock().unwrap();
        let s = lock.as_ref().unwrap();
        s.classes[&class].superclass
    };
    if superclass != 0 {
        initialize_class(superclass);
    }
    let call = loop {
        let mut lock = mutex.lock().unwrap();
        let s = lock.as_mut().unwrap();
        match s.initializing.get(&class) {
            Some(None) => return,
            Some(Some(owner)) if *owner == thread => return,
            Some(Some(_)) => {
                drop(wake.wait(lock).unwrap());
                continue;
            }
            None => {}
        }
        s.initializing.insert(class, Some(thread));
        let selector = s.intern(b"initialize".to_vec());
        break s
            .lookup(s.classes[&class].isa, b"initialize")
            .map(|imp| (imp, selector));
    };
    if let Some((imp, selector)) = call {
        unsafe {
            call_void(imp, class, selector);
        }
    }
    let mut lock = mutex.lock().unwrap();
    lock.as_mut().unwrap().initializing.insert(class, None);
    wake.notify_all();
}

// Called by the argument-preserving assembly trampolines below.
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn adapter_objc_lookup(
    receiver: u64,
    selector: u64,
    super_class: u64,
) -> u64 {
    let result = (|| -> Result<u64> {
        let (class, target, name) = {
            let lock = registry()
                .0
                .lock()
                .map_err(|_| "Objective-C registry poisoned")?;
            let s = lock.as_ref().ok_or("Objective-C runtime not active")?;
            let class = if s.classes.contains_key(&receiver) {
                receiver
            } else {
                s.object_class(receiver)?
            };
            let target = if super_class == 0 {
                s.object_class(receiver)?
            } else {
                s.classes
                    .get(&super_class)
                    .ok_or("unknown Objective-C super class")?
                    .superclass
            };
            let initialized = if s.classes[&class].meta {
                s.classes
                    .iter()
                    .find(|(_, v)| !v.meta && v.isa == class)
                    .map(|(p, _)| *p)
                    .ok_or("metaclass has no class")?
            } else {
                class
            };
            (initialized, target, s.string(selector)?)
        };
        initialize_class(class);
        let lock = registry().0.lock().unwrap();
        lock.as_ref()
            .unwrap()
            .lookup(target, &name)
            .ok_or_else(|| format!("unrecognized selector {}", String::from_utf8_lossy(&name)))
    })();
    result.unwrap_or_else(|e| fatal(e))
}

unsafe extern "sysv64" fn sel_register_name(name: *const c_char) -> u64 {
    let mut lock = registry().0.lock().unwrap();
    let s = lock
        .as_mut()
        .unwrap_or_else(|| fatal("Objective-C runtime not active"));
    let name = s.string(name as u64).unwrap_or_else(|e| fatal(e));
    s.intern(name)
}
unsafe extern "sysv64" fn sel_get_name(selector: u64) -> *const c_char {
    if selector == 0 {
        return c"<null selector>".as_ptr();
    }
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    let name = s.string(selector).unwrap_or_else(|e| fatal(e));
    s.intern(name) as *const c_char
}
unsafe extern "sysv64" fn sel_is_equal(a: u64, b: u64) -> i8 {
    if a == b {
        return 1;
    }
    if a == 0 || b == 0 {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    i8::from(s.string(a).unwrap_or_else(|e| fatal(e)) == s.string(b).unwrap_or_else(|e| fatal(e)))
}
unsafe extern "sysv64" fn get_class(name: *const c_char) -> u64 {
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    let name = s.string(name as u64).unwrap_or_else(|e| fatal(e));
    s.names.get(&name).copied().unwrap_or(0)
}
unsafe extern "sysv64" fn object_get_class(object: u64) -> u64 {
    if object == 0 {
        return 0;
    }
    registry()
        .0
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .object_class(object)
        .unwrap_or_else(|e| fatal(e))
}
unsafe extern "sysv64" fn class_get_superclass(class: u64) -> u64 {
    if class == 0 {
        return 0;
    }
    registry()
        .0
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .classes
        .get(&class)
        .unwrap_or_else(|| fatal("unknown Objective-C class"))
        .superclass
}
unsafe extern "sysv64" fn class_get_name(class: u64) -> *const c_char {
    if class == 0 {
        return c"nil".as_ptr();
    }
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    let name = s
        .classes
        .get(&class)
        .unwrap_or_else(|| fatal("unknown Objective-C class"))
        .name
        .clone();
    s.intern(name) as *const c_char
}
unsafe extern "sysv64" fn class_get_instance_size(class: u64) -> usize {
    if class == 0 {
        return 0;
    }
    registry()
        .0
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .classes
        .get(&class)
        .unwrap_or_else(|| fatal("unknown Objective-C class"))
        .size
}
unsafe extern "sysv64" fn class_is_meta_class(class: u64) -> i8 {
    if class == 0 {
        return 0;
    }
    i8::from(
        registry()
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .classes
            .get(&class)
            .unwrap_or_else(|| fatal("unknown Objective-C class"))
            .meta,
    )
}
unsafe extern "sysv64" fn class_create_instance(class: u64, extra: usize) -> u64 {
    if class == 0 {
        return 0;
    }
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    let data = s
        .classes
        .get(&class)
        .unwrap_or_else(|| fatal("unknown Objective-C class"));
    if data.meta {
        return 0;
    }
    let Some(size) = data
        .size
        .max(16)
        .checked_add(extra)
        .filter(|n| *n <= 1024 * 1024)
    else {
        return 0;
    };
    if s.allocations.len() >= 65536 {
        return 0;
    }
    let layout = Layout::from_size_align(size, 16).unwrap();
    let object = unsafe { alloc_zeroed(layout) };
    if object.is_null() {
        return 0;
    }
    unsafe {
        (object as *mut u64).write(class);
    }
    s.allocations.insert(
        object as u64,
        Allocation {
            layout,
            references: 1,
        },
    );
    object as u64
}
unsafe fn send_object(receiver: u64, name: &[u8]) -> u64 {
    if receiver == 0 {
        return 0;
    }
    let selector = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .intern(name.to_vec());
    let imp = unsafe { adapter_objc_lookup(receiver, selector, 0) };
    let function: unsafe extern "sysv64" fn(u64, u64) -> u64 =
        unsafe { std::mem::transmute(imp as usize) };
    unsafe { function(receiver, selector) }
}
unsafe extern "sysv64" fn objc_alloc(class: u64) -> u64 {
    // Compiler lowering of [Class alloc] must retain the guest's +alloc
    // override and lazy +initialize behavior, rather than allocating directly.
    unsafe { send_object(class, b"alloc") }
}
unsafe extern "sysv64" fn objc_alloc_init(class: u64) -> u64 {
    let object = unsafe { objc_alloc(class) };
    unsafe { send_object(object, b"init") }
}
unsafe extern "sysv64" fn object_dispose(object: u64) -> u64 {
    if object == 0 {
        return 0;
    }
    let allocation = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .allocations
        .remove(&object)
        .unwrap_or_else(|| fatal("object_dispose requires runtime-owned instance"));
    unsafe {
        dealloc(object as *mut u8, allocation.layout);
    }
    0
}
unsafe extern "sysv64" fn retain(object: u64) -> u64 {
    if object == 0 {
        return 0;
    }
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    if s.classes.contains_key(&object) {
        return object;
    }
    let allocation = s
        .allocations
        .get_mut(&object)
        .unwrap_or_else(|| fatal("retain requires runtime-owned instance"));
    allocation.references = allocation
        .references
        .checked_add(1)
        .unwrap_or_else(|| fatal("Objective-C retain overflow"));
    object
}
unsafe extern "sysv64" fn release(object: u64) {
    if object == 0 {
        return;
    }
    let action = {
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
        let class = s.object_class(object).unwrap_or_else(|e| fatal(e));
        let selector = s.intern(b"dealloc".to_vec());
        s.lookup(class, b"dealloc").map(|imp| (imp, selector))
    };
    if let Some((imp, selector)) = action {
        unsafe {
            call_void(imp, object, selector);
        }
    }
    let allocation = registry()
        .0
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .allocations
        .remove(&object);
    if let Some(allocation) = allocation {
        unsafe {
            dealloc(object as *mut u8, allocation.layout);
        }
    }
}
thread_local! { static POOLS: RefCell<Vec<Vec<u64>>> = const { RefCell::new(Vec::new()) }; }
unsafe extern "sysv64" fn pool_push() -> u64 {
    POOLS.with(|p| {
        let mut p = p.borrow_mut();
        p.push(Vec::new());
        p.len() as u64
    })
}
unsafe extern "sysv64" fn autorelease(object: u64) -> u64 {
    if object != 0 {
        POOLS.with(|p| {
            p.borrow_mut()
                .last_mut()
                .unwrap_or_else(|| fatal("autorelease requires an explicit pool"))
                .push(object)
        });
    }
    object
}
unsafe extern "sysv64" fn pool_pop(token: u64) {
    let objects = POOLS.with(|p| {
        let mut p = p.borrow_mut();
        if token == 0 || token != p.len() as u64 {
            fatal("autorelease pools must pop in stack order");
        }
        p.pop().unwrap()
    });
    for object in objects.into_iter().rev() {
        unsafe {
            release(object);
        }
    }
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("dispatch.s"));
#[cfg(target_arch = "x86_64")]
unsafe extern "sysv64" {
    fn adapter_objc_msg_send();
    fn adapter_objc_msg_send_super2();
}
pub fn symbol(name: &str) -> Option<u64> {
    static EMPTY_CACHE: [u64; 2] = [0; 2];
    Some(match name {
        "__objc_empty_cache" => EMPTY_CACHE.as_ptr() as u64,
        #[cfg(target_arch = "x86_64")]
        "_objc_msgSend" => adapter_objc_msg_send as *const () as u64,
        #[cfg(target_arch = "x86_64")]
        "_objc_msgSendSuper2" => adapter_objc_msg_send_super2 as *const () as u64,
        "_sel_registerName" | "_sel_getUid" => sel_register_name as *const () as u64,
        "_sel_getName" => sel_get_name as *const () as u64,
        "_sel_isEqual" => sel_is_equal as *const () as u64,
        "_objc_getClass" | "_objc_lookUpClass" => get_class as *const () as u64,
        "_object_getClass" => object_get_class as *const () as u64,
        "_class_getSuperclass" => class_get_superclass as *const () as u64,
        "_class_getName" => class_get_name as *const () as u64,
        "_class_getInstanceSize" => class_get_instance_size as *const () as u64,
        "_class_isMetaClass" => class_is_meta_class as *const () as u64,
        "_class_createInstance" => class_create_instance as *const () as u64,
        "_objc_alloc" => objc_alloc as *const () as u64,
        "_objc_alloc_init" => objc_alloc_init as *const () as u64,
        "_object_dispose" => object_dispose as *const () as u64,
        "_objc_retain" => retain as *const () as u64,
        "_objc_release" => release as *const () as u64,
        "_objc_autorelease" => autorelease as *const () as u64,
        "_objc_autoreleasePoolPush" => pool_push as *const () as u64,
        "_objc_autoreleasePoolPop" => pool_pop as *const () as u64,
        // No fpret/stret, weak, ARC return-value handshake, exceptions, protocols,
        // dynamic class mutation, tagged pointers or nonpointer isa adapters.
        _ => return None,
    })
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    unsafe extern "sysv64" fn mixed(
        _receiver: u64,
        _selector: u64,
        a: u64,
        b: f64,
        c: u64,
        d: u64,
        e: u64,
        f: u64,
        g: f64,
    ) -> f64 {
        (a + c + d + e + f) as f64 + b + g
    }

    struct Fixture {
        data: Vec<u64>,
        strings: Vec<CString>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut data = vec![0u64; 40];
            let strings = vec![
                CString::new("UnitRoot").unwrap(),
                CString::new("mixed:").unwrap(),
                CString::new("d80@0:8Q16d24Q32Q40Q48Q56d64").unwrap(),
            ];
            let p = data.as_ptr() as u64;
            data[0] = p + 5 * 8; // class isa
            data[4] = p + 10 * 8; // class ro
            data[5] = p + 5 * 8; // root metaclass isa
            data[6] = p; // root metaclass superclass is the root class
            data[9] = p + 19 * 8;
            data[10] = 2; // root class
            data[11] = 8; // instanceSize
            data[13] = strings[0].as_ptr() as u64;
            data[14] = p + 28 * 8;
            data[19] = 3 | (40 << 32); // meta + root, instanceStart40
            data[20] = 40;
            data[22] = strings[0].as_ptr() as u64;
            data[28] = 24 | (1 << 32); // method entsize/count
            data[29] = strings[1].as_ptr() as u64;
            data[30] = strings[2].as_ptr() as u64;
            data[31] = mixed as *const () as u64;
            data[32] = p;
            data[33] = strings[1].as_ptr() as u64;
            data[34] = 64 << 32; // imageinfo version0, flags64
            Self { data, strings }
        }
        fn image(&self) -> Image {
            let p = self.data.as_ptr() as u64;
            let mut regions = vec![Region {
                address: p,
                size: (self.data.len() * 8) as u64,
                readable: true,
                writable: true,
                executable: false,
            }];
            regions.extend(self.strings.iter().map(|s| Region {
                address: s.as_ptr() as u64,
                size: s.as_bytes_with_nul().len() as u64,
                readable: true,
                writable: false,
                executable: false,
            }));
            regions.push(Region {
                address: mixed as *const () as u64,
                size: 1,
                readable: true,
                writable: false,
                executable: true,
            });
            let sections = [
                ("__objc_classlist", 32),
                ("__objc_selrefs", 33),
                ("__objc_imageinfo", 34),
            ]
            .into_iter()
            .map(|(name, offset)| MetadataSection {
                name: name.into(),
                address: p + offset * 8,
                size: 8,
            })
            .collect();
            Image { regions, sections }
        }
    }

    #[test]
    fn validates_atomically_and_dispatches_registers_stack_and_nil() {
        let mut fixture = Fixture::new();
        let original_selector = fixture.data[33];
        // A malformed image must neither become active nor patch a selector.
        fixture.data[31] = 1;
        assert!(begin(vec![fixture.image()]).is_err());
        assert_eq!(fixture.data[33], original_selector);
        fixture.data[31] = mixed as *const () as u64;
        let guard = begin(vec![fixture.image()]).unwrap();
        guard.initialize().unwrap();
        assert_ne!(fixture.data[33], original_selector);
        let class = fixture.data.as_ptr() as u64;
        let object = unsafe { class_create_instance(class, 0) };
        assert_eq!(unsafe { object_get_class(object) }, class);
        let send: unsafe extern "sysv64" fn(u64, u64, u64, f64, u64, u64, u64, u64, f64) -> f64 =
            unsafe { std::mem::transmute(adapter_objc_msg_send as *const ()) };
        assert_eq!(
            unsafe { send(object, fixture.data[33], 1, 2.0, 3, 4, 5, 6, 7.0) },
            28.0
        );
        assert_eq!(
            unsafe { send(0, fixture.data[33], 1, 2.0, 3, 4, 5, 6, 7.0) },
            0.0
        );
        unsafe {
            object_dispose(object);
        }
        drop(guard);
        // ARC metadata fails before any registration or selector patch.
        fixture.data[33] = original_selector;
        fixture.data[10] |= 0x80;
        assert!(begin(vec![fixture.image()]).is_err());
        assert_eq!(fixture.data[33], original_selector);
    }
}
