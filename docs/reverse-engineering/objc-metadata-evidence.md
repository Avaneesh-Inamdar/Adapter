# Objective-C protocol and property evidence

Recorded 2026-10-22. This slice uses Apple source definitions, LLVM-emitted
metadata and execution of the identical Mac-built Mach-O on both hosts. No Apple
runtime implementation was copied into Adapter.

## Pinned definitions

Apple objc4 revision `fb265098298302243cd7eeaa1f63f0ba7786dd9a` defines
[`protocol_t`, `protocol_list_t` and `property_t`](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-runtime-new.h).
The same revision's
[`objc-runtime-new.mm`](https://github.com/apple-oss-distributions/objc4/blob/fb265098298302243cd7eeaa1f63f0ba7786dd9a/runtime/objc-runtime-new.mm)
defines the query behavior. The important distinctions are that protocol method
lookup recurses through adopted protocols, protocol optional-property queries
return nil, and C `class_conformsToProtocol` does not walk superclasses. The
NSObject conformance method has a separate superclass traversal.

## Compiler observation

The source is [objc_metadata.m](../../tests/fixtures/objc_metadata.m), SHA-256
`c1bb41d397c55ccf5bdc078e6a93856f7ab53d016c9b9b183f8a07f59c5ad007`.
LLVM 23.1.3 on Windows, targeting Darwin x86-64 10.13, emits:

- 96-byte protocol records, with null isa, size 96, flags 0, four method-list
  pointers, instance properties, extended type array and class properties.
- Protocol method lists with entry size 24 and null IMPs, separate from
  the executable class/category method lists.
- 16-byte property entries holding pointers to name and attributes.
- Pointer-sized adoption counts and compiler reference sections
  `__objc_protolist` / `__objc_protorefs`.
- Category instance and class property pointers; image-info bit 6 enables the
  class-property field.

The IR command was:

```powershell
& 'C:\Program Files\LLVM\bin\clang.exe' -target x86_64-apple-macos10.13 -O0 -fno-objc-arc -S -emit-llvm tests/fixtures/objc_metadata.m -o target/objc-metadata.ll
```

| Generated artifact | SHA-256 |
| --- | --- |
| LLVM IR from the command above | `903032679ca29bba04fb8ce3a310ce8a32f3686ed069ebaea329291db51b56cb` |
| Fixture object | `83a5959eaf46065ff27c1fc352caa0e26c6762e04c363a4535e176e5373603d5` |
| Classic executable | `3b87b9c0757a01b4cc66893014a0d06b32a3967e4973be09047f988933e1c58c` |
| Chained executable, absolute methods | `9df89c2976b193a3f81c9216753592691d5567a67f5ac34494e19b8992b92a6f` |
| Chained executable, relative methods | `df4384b2c0fb5381f77bbc3aca5520ee2661d8f1fd3f8d67492bb93bc46251a6` |

The normal fixture has an observable C constructor. Mutating a protocol size,
flag, adoption count/cycle, description IMP or property count must exit 125
before that marker. The final debug and release suites verify these failures.

## Native reference

[Mac run 39805811658](https://github.com/Avaneesh-Inamdar/Adapter/actions/runs/39805811658)
built the same source with Apple clang 17.0.0 on Intel Darwin 24.6.0. The downloaded
executable SHA-256 is
`49f60699f0f770d29375b7f837c3b435f26b8fc09924201776f3169730dd5c7e`.
All 4,096 records match in debug and release. This also verifies the 16-byte
`objc_method_description` result across the SysV-to-host adapter boundary.

This is one metadata graph, not thousands of different class layouts. Most
families repeat fixed queries; accessor arguments vary by record index. Object
ownership, atomic properties, ARC/weak, Foundation, protocol messages and dynamic
metadata remain outside this evidence. See [contract v3](../abi/objc-abi.md) and
[retained captures](../differential-testing.md#recorded-objective-c-metadata-run-2026-10-22).
