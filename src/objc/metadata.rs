//! Compiler-emitted protocol/property records and read-only runtime queries.
use super::*;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct MethodDescription {
    name: u64,
    types: u64,
}
pub(super) struct Property {
    name: u64,
    attributes: u64,
}
pub(super) struct Protocol {
    name: Vec<u8>,
    inherited: Vec<u64>,
    methods: [BTreeMap<Vec<u8>, MethodDescription>; 4],
    properties: [BTreeMap<Vec<u8>, u64>; 2],
}

impl Session {
    pub(super) fn read_properties(&mut self, list: u64) -> Result<BTreeMap<Vec<u8>, u64>> {
        let mut properties = BTreeMap::new();
        if list == 0 {
            return Ok(properties);
        }
        if self.u32(list)? != 16 {
            return Err("Objective-C property entry size unsupported".into());
        }
        let count = self.u32(list + 4)? as u64;
        if count > 65536 || self.properties.len() as u64 + count > 65536 {
            return Err("Objective-C property limit exceeded".into());
        }
        self.range(list + 8, count * 16, false)?;
        for index in 0..count {
            let address = list + 8 + index * 16;
            let name = self.u64(address)?;
            let attributes = self.u64(address + 8)?;
            let text = self.string(name)?;
            if text.is_empty()
                || self.string(attributes)?.is_empty()
                || properties.insert(text, address).is_some()
            {
                return Err("Objective-C empty or duplicate property metadata".into());
            }
            self.properties
                .insert(address, Property { name, attributes });
        }
        Ok(properties)
    }

    pub(super) fn read_protocol_list(
        &mut self,
        list: u64,
        depth: usize,
        active: &mut BTreeSet<u64>,
    ) -> Result<Vec<u64>> {
        if list == 0 {
            return Ok(Vec::new());
        }
        let count = self.u64(list)?;
        if count > 8192 {
            return Err("Objective-C protocol list limit exceeded".into());
        }
        self.protocol_references += count;
        if self.protocol_references > 65536 {
            return Err("Objective-C total protocol reference limit exceeded".into());
        }
        self.range(list + 8, count * 8, false)?;
        let mut protocols = Vec::new();
        for index in 0..count {
            let protocol = self.u64(list + 8 + index * 8)?;
            self.read_protocol(protocol, depth, active)?;
            protocols.push(protocol);
        }
        Ok(protocols)
    }

    pub(super) fn read_protocol(
        &mut self,
        address: u64,
        depth: usize,
        active: &mut BTreeSet<u64>,
    ) -> Result<()> {
        if active.contains(&address) {
            return Err("Objective-C protocol inheritance cycle".into());
        }
        if self.protocols.contains_key(&address) {
            return Ok(());
        }
        if depth > 128 || self.protocols.len() + active.len() >= 8192 {
            return Err("Objective-C protocol graph limit exceeded".into());
        }
        self.range(address, 72, false)?;
        let size = self.u32(address + 64)?;
        if ![72, 80, 88, 96].contains(&size)
            || self.u32(address + 68)? != 0
            || self.u64(address)? != 0
        {
            return Err(
                "Objective-C protocol size, flags or preinitialized isa unsupported".into(),
            );
        }
        self.range(address, u64::from(size), false)?;
        let name = self.string(self.u64(address + 8)?)?;
        if name.is_empty()
            || self
                .protocol_names
                .get(&name)
                .is_some_and(|p| *p != address)
        {
            return Err("Objective-C empty or duplicate protocol name".into());
        }
        active.insert(address);
        let inherited = self.read_protocol_list(self.u64(address + 16)?, depth + 1, active)?;
        let mut methods = std::array::from_fn(|_| BTreeMap::new());
        let mut total = 0;
        for (index, entries) in methods.iter_mut().enumerate() {
            let list = self.u64(address + 24 + index as u64 * 8)?;
            if list == 0 {
                continue;
            }
            if self.u32(list)? != 24 {
                return Err("Objective-C protocol method entry size unsupported".into());
            }
            let count = self.u32(list + 4)? as u64;
            if count > 65536 {
                return Err("Objective-C protocol method limit exceeded".into());
            }
            self.protocol_methods += count;
            if self.protocol_methods > 65536 {
                return Err("Objective-C total protocol method limit exceeded".into());
            }
            self.range(list + 8, count * 24, false)?;
            total += count;
            for index in 0..count {
                let item = list + 8 + index * 24;
                let selector = self.string(self.u64(item)?)?;
                let types = self.u64(item + 8)?;
                if selector.is_empty()
                    || self.string(types)?.is_empty()
                    || self.u64(item + 16)? != 0
                {
                    return Err("Objective-C invalid protocol method description".into());
                }
                let description = MethodDescription {
                    name: self.intern(selector.clone()),
                    types,
                };
                if entries.insert(selector, description).is_some() {
                    return Err("Objective-C duplicate protocol method".into());
                }
            }
        }
        let instance = self.read_properties(self.u64(address + 56)?)?;
        let class = if size >= 96 {
            self.read_properties(self.u64(address + 88)?)?
        } else {
            BTreeMap::new()
        };
        if size >= 80 {
            let extended = self.u64(address + 72)?;
            if extended != 0 {
                self.range(extended, total * 8, false)?;
                for index in 0..total {
                    self.string(self.u64(extended + index * 8)?)?;
                }
            }
        }
        if size >= 88 && self.u64(address + 80)? != 0 {
            return Err("Objective-C demangled Swift protocols unsupported".into());
        }
        active.remove(&address);
        if self
            .protocol_names
            .get(&name)
            .is_some_and(|p| *p != address)
        {
            return Err("Objective-C duplicate protocol name".into());
        }
        self.protocol_names.insert(name.clone(), address);
        self.protocols.insert(
            address,
            Protocol {
                name,
                inherited,
                methods,
                properties: [instance, class],
            },
        );
        Ok(())
    }

    // Prevalidated acyclic graphs; visited sets also bound diamond traversals.
    fn protocol_walk(&self, root: u64) -> Vec<u64> {
        let mut stack = vec![root];
        let mut visited = BTreeSet::new();
        let mut output = Vec::new();
        while let Some(address) = stack.pop() {
            if !visited.insert(address) {
                continue;
            }
            let protocol = self
                .protocols
                .get(&address)
                .unwrap_or_else(|| fatal("unknown Objective-C protocol"));
            output.push(address);
            stack.extend(protocol.inherited.iter().rev().copied());
        }
        output
    }
    fn protocol_conforms(&self, root: u64, other: u64) -> bool {
        if root == 0 || other == 0 {
            return false;
        }
        if !self.protocols.contains_key(&other) {
            fatal("unknown Objective-C protocol");
        }
        self.protocol_walk(root).contains(&other)
    }
}

unsafe extern "sysv64" fn get_protocol(name: *const c_char) -> u64 {
    if name.is_null() {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    let name = s.string(name as u64).unwrap_or_else(|e| fatal(e));
    s.protocol_names.get(&name).copied().unwrap_or(0)
}
unsafe extern "sysv64" fn protocol_name(protocol: u64) -> *const c_char {
    if protocol == 0 {
        return c"nil".as_ptr();
    }
    let mut lock = registry().0.lock().unwrap();
    let s = lock.as_mut().unwrap();
    let name = s
        .protocols
        .get(&protocol)
        .unwrap_or_else(|| fatal("unknown Objective-C protocol"))
        .name
        .clone();
    s.intern(name) as *const c_char
}
unsafe extern "sysv64" fn protocol_conforms(root: u64, other: u64) -> i8 {
    let lock = registry().0.lock().unwrap();
    i8::from(lock.as_ref().unwrap().protocol_conforms(root, other))
}
unsafe extern "sysv64" fn protocol_equal(a: u64, b: u64) -> i8 {
    if a == b {
        return 1;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    i8::from(s.protocol_conforms(a, b) && s.protocol_conforms(b, a))
}
unsafe extern "sysv64" fn class_conforms(class: u64, protocol: u64) -> i8 {
    if class == 0 || protocol == 0 {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    // The C runtime API checks this class only. NSObject's -conformsToProtocol:
    // separately searches superclasses; that method is not supplied here.
    let class = s
        .classes
        .get(&class)
        .unwrap_or_else(|| fatal("unknown Objective-C class"));
    i8::from(
        class
            .protocols
            .iter()
            .any(|p| s.protocol_conforms(*p, protocol)),
    )
}
unsafe extern "sysv64" fn method_description(
    protocol: u64,
    selector: u64,
    required: i8,
    instance: i8,
) -> MethodDescription {
    if protocol == 0 || selector == 0 {
        return MethodDescription::default();
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    let name = s.string(selector).unwrap_or_else(|e| fatal(e));
    let list = usize::from(required == 0) * 2 + usize::from(instance == 0);
    for address in s.protocol_walk(protocol) {
        if let Some(description) = s.protocols[&address].methods[list].get(&name) {
            return *description;
        }
    }
    MethodDescription::default()
}
unsafe extern "sysv64" fn class_property(mut class: u64, name: *const c_char) -> u64 {
    if class == 0 || name.is_null() {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    let name = s.string(name as u64).unwrap_or_else(|e| fatal(e));
    while class != 0 {
        let data = s
            .classes
            .get(&class)
            .unwrap_or_else(|| fatal("unknown Objective-C class"));
        if let Some(property) = data.properties.get(&name) {
            return *property;
        }
        class = data.superclass;
    }
    0
}
unsafe extern "sysv64" fn protocol_property(
    protocol: u64,
    name: *const c_char,
    required: i8,
    instance: i8,
) -> u64 {
    if protocol == 0 || name.is_null() || required == 0 {
        return 0;
    }
    let lock = registry().0.lock().unwrap();
    let s = lock.as_ref().unwrap();
    let name = s.string(name as u64).unwrap_or_else(|e| fatal(e));
    for address in s.protocol_walk(protocol) {
        if let Some(property) =
            s.protocols[&address].properties[usize::from(instance == 0)].get(&name)
        {
            return *property;
        }
    }
    0
}
unsafe extern "sysv64" fn property_name(property: u64) -> *const c_char {
    if property == 0 {
        return std::ptr::null();
    }
    let lock = registry().0.lock().unwrap();
    lock.as_ref()
        .unwrap()
        .properties
        .get(&property)
        .unwrap_or_else(|| fatal("unknown Objective-C property"))
        .name as *const c_char
}
unsafe extern "sysv64" fn property_attributes(property: u64) -> *const c_char {
    if property == 0 {
        return std::ptr::null();
    }
    let lock = registry().0.lock().unwrap();
    lock.as_ref()
        .unwrap()
        .properties
        .get(&property)
        .unwrap_or_else(|| fatal("unknown Objective-C property"))
        .attributes as *const c_char
}
pub(super) fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_objc_getProtocol" => get_protocol as *const () as u64,
        "_protocol_getName" => protocol_name as *const () as u64,
        "_protocol_isEqual" => protocol_equal as *const () as u64,
        "_protocol_conformsToProtocol" => protocol_conforms as *const () as u64,
        "_class_conformsToProtocol" => class_conforms as *const () as u64,
        "_protocol_getMethodDescription" => method_description as *const () as u64,
        "_class_getProperty" => class_property as *const () as u64,
        "_protocol_getProperty" => protocol_property as *const () as u64,
        "_property_getName" => property_name as *const () as u64,
        "_property_getAttributes" => property_attributes as *const () as u64,
        _ => return None,
    })
}
