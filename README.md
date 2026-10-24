# xt_stable_abi

**English** · [中文](README.zh-CN.md)

Types, ownership and virtual tables for exchanging values between two
independently built Rust binaries.

Two programs built by different compiler versions, with different feature
unification and different dependency graphs, do not share a Rust ABI. This crate
gives them one: every value that crosses the boundary has a declared byte layout,
every owned allocation carries the allocator that produced it, and every dynamic
call goes through a table whose first fields say what the table is.

Three operational notes - what a boundary costs when it is called millions of
times, what a published library exposes of the code behind it, and how a
deployment may ship it - are collected in
[`docs/boundary-guidance.md`](docs/boundary-guidance.md), with the measurements
they rest on.

The third one is a hard rule and is repeated here: **ship the dynamic library
only.** An `rlib` carries `.rmeta`, which holds source-level information
including doc comments; a `.pdb` carries every Rust symbol name. Neither may be
published, and a build directory may not be archived as a whole.

## The rules

- **Explicit layout.** Boundary types are `#[repr(C)]`, and every width is
  written out (`u64`, `u32`) rather than taken from `usize`.
- **Explicit tags.** `Option` and `Result` share bytes with a niche under a
  given compiler; the types here start with a `u32` tag, and the payload storage
  of an empty value is zeroed, so an empty value does not carry the caller's
  stack contents. Bytes the compiler inserts as padding between fields are not
  written: they are not part of the value, and a value must not be compared or
  stored by its raw bytes.
- **Owned values carry their allocator.** `StableString`, `StableVec` and
  friends store a pointer, a length, a capacity and an [`AbiAllocator`]. Dropping
  a value on the receiving side calls the functions that produced the block, so
  one side may hand memory to a host that uses a different allocator.
- **Table headers identify the declaration.** A generated table starts with
  `type_id`, `signature`, `layout_revision`, `size`, `align`, the number of
  slots that follow and a drop function, and is followed by one entry per
  method. Each entry carries the identifier of the method it calls, so a
  receiver finds the slot it needs by comparing numbers rather than by trusting
  a position, and reads at most the number of slots the header records. The
  entries of a table this crate builds are ordered by that identifier, so a call
  on a long table is a binary search; a short one is walked, and a table written
  by hand is found by the walk that follows a miss. A table
  built from a different declaration, or against a different boundary layout, is
  rejected instead of being called through the wrong slot. The signature folds
  the identifiers of the methods the build compiled, so a method a `#[cfg]`
  removed changes the number. Each type a method mentions has a number of its
  own, [`AbiIdentity::IDENTIFIER`], and a method's identifier is folded from the
  numbers of the types it mentions rather than from the way they were written:
  `StableStr`, `StableStr<'_>` and `xt_stable_abi::StableStr` are one type, so
  they name one identifier, and a local alias of a boundary type names that type
  too. A type outside this crate takes part by implementing [`AbiIdentity`] —
  `unsafe_stable_identity!` writes the implementation for a name — and the kinds
  a container folds in are part of the protocol for a table written by hand. The
  fold is
  commutative: the order the methods are listed in is not part of the
  declaration, because a call finds its slot by comparing identifiers rather
  than by counting entries.
- **Calls declare boundary types.** Arguments and return types must implement
  [`AbiType`]; a declaration that names `String`, `usize` or `bool` fails to
  compile. A returned value is checked by [`AbiType::validate`] at the accessor
  before safe code sees it: a tag outside the set the type defines, text that is
  not UTF-8, a vector whose length is past its capacity, a map whose keys are
  out of order, or an object whose table and address disagree are all refused
  there. An argument is checked too, on the side that can report it: the
  accessor of a table built with `stable_object!` checks each argument before the
  call, abandoning the value and panicking when it fails, because that table has
  no status word to report a refusal through; a table built with
  `stable_object_guarded!` checks the arguments in its trampoline, before the
  implementation sees them, and reports [`AbiStatus::InvalidArgument`].
- **A contained panic travels with the call.** `stable_object_guarded!` passes a
  status word from the caller into the trampoline, so the report survives the
  case where the implementation lives in another dynamic library, where every
  thread-local would be a separate copy. The word distinguishes the two
  failures: a panic is [`AbiStatus::Panic`], while an argument the receiving side
  refuses is [`AbiStatus::InvalidArgument`], which says the implementation was
  never entered. An argument that was refused is abandoned rather than
  released, because its bytes do not describe an owner the receiving side may
  hand back; the arguments that passed their check are released normally, since
  abandoning them would leak what the sender handed over. A method that takes
  the receiver by value keeps the receiver, which its own drop then releases.
  The panic hook still runs: `catch_unwind` stops the unwind, not the default
  hook, so an implementation that must not write its panic text to the process's
  standard error stream has to install a hook of its own.
- **A method may be older than its peer.** Strict adoption requires every
  declared method to be present. `abi_try_from_stable_compatible` accepts a peer
  whose method list is a revision of this one - it declares a subset or a
  superset of the same methods, with no method named twice. A list that shares no
  whole declaration with it, or that names one method twice, belongs to another
  trait or has displaced a method, and is refused instead of being called
  through.
- **Versions are negotiated, not assumed.** An [`AbiVersion`] serves a request
  made by the same major version and an equal or older minor version. A host
  therefore accepts a library at a newer minor than its own - the major number
  and the layout revision are what decide whether the fields can be read - and
  refuses one older than the version it requires.

## Method shapes

`stable_object!` and `stable_object_guarded!` accept:

- `fn name(&self)`, `fn name(&mut self)` and `fn name(self)`;
- any number of `argument: Type` pairs after the receiver, optionally ended by a
  comma;
- an optional `-> Type`, where `-> ()` names the same declaration as no return
  type at all;
- a trailing `;` or a default body in braces. A default body is part of the
  trait, so an implementation that does not override it still answers through
  the slot;
- attributes on the method, including `#[cfg]`: the trait item, the table
  entry, the accessor and the trampoline are gated together.

A generic method and an associated type or constant name shapes one fixed slot
cannot carry; they are reported at the declaration with a message that says so.
Writing the receiver's lifetime out - `fn peek<'a>(&'a self)` - is that same
generic method, and is refused there too: the receiver is spelled `&self`,
`&mut self` or `self`. A return type that names a lifetime, and `-> Self`, are
refused there as well: a slot lends its value only for the call, so a returned
borrow would outlive the storage, and `Self` names the implementation type, which
the caller does not know. `'static` is refused under the same rule: the storage
of a dynamic library does not outlive the library, and the type system is not told
when it is unloaded. Return an owned value instead. A borrowed type nested
inside the return type (`StableOption<StableStr>`) is refused by the same rule,
because the borrow it names is the one the slot cannot lend.

A declaration cannot name a supertrait either. The table lists the methods of
the trait being declared, so an inherited method would have no slot; repeat the
inherited methods in this trait, or cross the boundary with an object whose own
declaration lists them. The macro reports that shape at the declaration instead
of failing later on a trait that compiled but could not answer a call.

A method that varies only in its argument types is written as one trait method
per instantiation, with the generic one kept on the implementation
(`tests/objects.rs` holds this shape):

```rust
stable_object! {
    pub struct ConverterObject;
    pub struct ConverterVTable;
    pub trait Converter {
        fn identity_u32(&self, value: u32) -> u32;
        fn identity_u64(&self, value: u64) -> u64;
    }
}

impl PassThrough {
    fn identity<T>(&self, value: T) -> T {
        value
    }
}

impl Converter for PassThrough {
    fn identity_u32(&self, value: u32) -> u32 {
        self.identity(value)
    }
    fn identity_u64(&self, value: u64) -> u64 {
        self.identity(value)
    }
}
```

## Types

Scalars and their checked wrappers ([`StableBool`], [`StableChar`]), text
([`StableStr`], [`StableString`], [`StableCowStr`]), sequences ([`StableVec`],
[`StableSlice`], [`StableSliceMut`], [`StableArray`], [`StableCowSlice`]),
[`StableBox`], tagged values ([`StableOption`], [`StableResult`]), ordered
collections ([`StableMap`], [`StableSet`]), collections ordered by a hash the
caller supplies ([`StableHashMap`], [`StableHashSet`]), groups ([`StablePair`],
[`StableTriple`], [`StableQuad`]), file names and paths ([`StableOsStr`],
[`StableOsString`], [`StablePath`], [`StablePathBuf`]), time ([`StableDuration`],
[`StableSystemTime`], [`StableInstant`]), names for entries ([`Handle`]) and
objects ([`StableDyn`] and the handles the macros generate). A domain
enumeration is declared with [`stable_enum!`], which writes the tag and payload
union out rather than deriving them; it also writes a `Wipe` for the type, so a
payload that cannot be wiped needs [`stable_enum_unwiped!`], which declares the
same enumeration without one. A declaration names the traits its values are to
have - `derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash, StableHash]` -
because a `derive` would read the union rather than the payload the tag selects;
`Clone` also writes [`TryClone`], `StableHash` makes the type usable as a key of
a hash map whose numbers this crate computes, and `default = Variant;` names the
variant a `Default` value is, which is what turns `Default` on.
Every owned type carries the allocator that produced it; every type that has
states the compiler would not catch validates them at the boundary. Copying is
available twice: `Clone` for a caller that accepts a panic when an allocation
fails, and [`TryClone`] for one that reports the failure instead. `TryClone` is
implemented for the scalar wrappers and the readings, for every owned container
(including the hash map and the hash set, whose copies keep the stored numbers),
and for the borrowed views, whose copies name the same storage. A writable view
has no implementation, because a second writable view of one block would let two
callers write through it at once; a caller that must hand one on moves it. An
object handle has no `Clone` either: the handle owns the value, so two of them
would release it twice. A side that needs a second name for one object passes the
handle on, or names it with a [`Handle`], which the table that owns the value can
hand out more than once.

A hash map is ordered by the numbers its entries carry, so the function that
produced them is part of what the two sides agree on. Passing one in
([`StableHashMap::insert`] and its siblings) makes that function yours and the
agreement explicit: the receiver's check for it is
[`StableHashMap::validate_with`]. [`StableHash`] is the other way: the number is
computed by this crate, so both sides agree by construction, and
[`StableHashMap::insert_stable`] with [`StableHashMap::validate_stable`] needs
nothing between the two sides.

A container of zero-sized elements carries nothing but its length, and nothing
bounds that number, so the boundary refuses one that holds elements: a sender
with nothing to carry sends the count as a number.

Sorting is a stable merge by default, taking a block of the same size as the
vector. [`StableVec::sort_unstable_by`] rearranges the block the vector already
owns instead: it allocates nothing, so it has no fallible form, and it gives up
the order of elements that compare equal.

## Example

```rust
use xt_stable_abi::{stable_object, StableStr, StableString};

stable_object! {
    /// A handle to a greeting source.
    pub struct GreeterObject;
    /// The table behind `GreeterObject`.
    pub struct GreeterVTable;
    /// Produces greetings.
    pub trait Greeter {
        /// Returns a greeting for `name`.
        fn greet(&self, name: StableStr<'_>) -> StableString;
    }
}

struct Plain;

impl Greeter for Plain {
    fn greet(&self, name: StableStr<'_>) -> StableString {
        StableString::from_str(&format!("Hello, {}", name.as_str())).unwrap()
    }
}

let object = GreeterObject::abi_from_value(Plain);
let greeting = unsafe { object.greet(StableStr::from_str("Ada")) };
assert_eq!(greeting.as_str(), "Hello, Ada");
```

The generated accessor is `unsafe` because its soundness rests on the other side
having been built from the same declaration; [`stable_object`] documents the
method shapes it accepts.

## The entry protocol

A host builds a [`HostServices`] (its own byte size, version, allocator, log
sink, context and a query function) and calls an entry point exported by the
library. The entry point returns a `u32` status code: a code this host does not
define is a failure, never an invalid enum value. The library fills a
[`PluginDescriptor`] whose first field is an [`AbiHeader`] carrying the buffer
capacity the host provided and the byte count the library wrote. [`load`]
validates the status, the byte count, the layout revision and the presence of an
object; [`LoadedPlugin`] releases what the library owns, then runs its shutdown
hook, then drops the keep-alive value the host supplied - which is the point at
which the host may unload the library.

A virtual table is built once per implementation type and shared by every value
of that type, so it outlives all of them and belongs to the allocator of the
process, not of the module that built it. A library that may be unloaded calls
[`release_cached_tables`] from its shutdown hook to give those tables back; the
next load builds its own. The call carries the obligations unloading already
carries: every object the library handed out has been released, and no thread is
inside a call through one of its tables.

The query function is the channel for everything beyond an allocator and a log
sink: the library asks for a service by identifier and reads the answer with the
layout the two sides agreed on. A host object, a callback table or a clock all
travel that way, and a plugin that has more than one object publishes them
through the object it hands over (an accessor that returns another
[`StableDyn`]).

```rust,ignore
// host side
let services = HostServices::new(AbiAllocator::system());
let entry = /* obtained from the loaded library */;
let mut plugin = LoadedPlugin::new(unsafe { load(entry, &services, ABI_VERSION) }?);
let object = MyObject::abi_try_from_stable(plugin.take_api())?;
```

The descriptor records `written` as the number of bytes the writer produced, and
`MIN_DESCRIPTOR_SIZE` is the size of a frozen declaration of the first layout,
so it does not grow when the descriptor gains a field. A library built
against that layout is accepted by a host that grew the structure later, because
the buffer the host provides is at least the size of its own layout and every
byte past `written` was written by the host.

The direction that does not follow is a library growing the descriptor. The
writer checks the buffer's capacity against its own layout before it writes
(`PluginDescriptor::store`), so a library that appended a field needs a host
whose buffer covers it; a host built against the older layout answers
`InvalidDescriptor`, which is a failure the entry point reports rather than a
version check the host gets to make. A release that appends a descriptor field is
therefore one the two sides have to agree on, not a change the minor number
carries on its own.

A byte count alone does not say where a field sits, so the header also records
`layout_revision`. A descriptor written against another revision is refused with
`VersionMismatch`, and because its fields may sit at other offsets it is neither
released nor called through - the same answer a descriptor that overstates its
own length gets. A descriptor from another major version is treated the same
way. A library at a newer minor is accepted; one older than the version the host
requires has its fields released and its hook run before the error is returned,
because its own report is still sound and only what it published is unusable.

## A worked example: kernel, surface, library, programs

`demo/` builds the shape this crate exists for, in the order a real project
builds it:

- `demo/kernel` (`xt_kernel`) is an ordinary library - a text-analysis engine -
  that knows nothing about this crate. It has its own 31 tests and uses the
  language broadly: borrowing iterators, generic heaps, `dyn` traits, `Cell` and
  `Mutex`, recursion, a union-find, a `const`-built CRC table, a transparent
  bit-set, an error enum with `source`, and a function-pointer transition table.
- `demo/api` (`xt_kernel_api`) is designed *from* that library: it names what a
  foreign caller may ask for and re-exports the boundary types. What the kernel
  keeps to itself - borrowed tokens, counted maps, the ranking heap, the error
  enum - never crosses. It also carries the loader, so a program can open a
  library with no operating-system code of its own.
- `demo/impl` (`xt_kernel_impl`) depends on the kernel and the surface and
  implements every published method. It is a library, so the same
  implementation can be linked into a program or compiled into a library.
- `demo/dynlib` (`xt_kernel_dynlib`) exports two names and calls into
  `xt_kernel_impl`. It builds a `cdylib` and nothing else: a deliverable must
  not also produce an `rlib`, whose metadata holds source-level information.
- `demo/app` (`xt_kernel_app`) depends on the surface only. It finds the library
  next to its own executable, loads it, and runs the scenarios over text, bytes,
  graphs, numbers, the build, sessions and shutdown.
- `demo/static` (`xt_kernel_static`) and `demo/native` (`xt_kernel_native`) run
  the same scenarios two other ways: against the same entry point with the
  implementation linked into the binary, and against the kernel with no boundary
  at all.
- `demo/bench` (`xt_bench`) and `demo/surface-checks` (`xt_surface_checks`) hold
  the inputs, the expected answers, the timed rounds and the scenario list, so
  the three programs differ in the path they take and in nothing else.

Build and run all three:

```sh
cargo build --workspace
cargo run -p xt_kernel_native
cargo run -p xt_kernel_static
cargo run -p xt_kernel_app
```

Each program fails with a non-zero exit code when any check fails, and prints the
time each scenario took, so the three can be put side by side: the reference, the
same boundary linked in, and the same boundary reached through a file.
`demo/README.md` states the dependency rule each crate keeps and which answer
differs between the reference and a caller that has only the published surface.

## What is verified

- `tests/` holds every test; there are none inside `src/`.
- `tests/layout.rs` reads live values through independent `#[repr(C)]` mirrors,
  so a field that moves fails a test rather than a downstream build.
- `tests/end_to_end.rs` loads two dynamic libraries: one built against this
  crate, and one (`plugins/legacy_plugin`) that writes the boundary types out by
  hand without depending on it. Both are loaded with `LoadLibraryW`/`dlopen`. The
  same file pins the parts that only a dynamic library can show: a panic caught
  inside the library still reaches the host, the shutdown hook runs after the
  objects are released, and the hand-written library refuses a buffer it cannot
  fill.
- `tests/table_release.rs` builds an object, drops it, and calls
  [`release_cached_tables`]. A global allocator that wraps the system one records
  that the table's own address comes back; a second call frees nothing, and the
  next value builds a new table.
- `demo/api/tests/unload.rs` loads the same library three times in one process,
  uses it, and asks the operating system whether the module was mapped before the
  drop and unmapped after it. A second, ignored test measures the private bytes
  the cycles leave behind.
- `tests/hostile.rs` feeds fixed malformed descriptors, tags and lengths to the
  receiving side and asserts the status each one produces.
- `tests/entry.rs` covers the descriptor protocol, including a writer that knows
  only the first layout, a rejected descriptor whose hook still runs, and the
  order in which the hook and the object release happen.
- `tests/validation.rs` builds values whose bytes say something the type does not
  define - an undefined tag over a payload of another type, a copy-on-write value
  whose tag names the other field, text that is not UTF-8, a vector whose length
  is past its capacity, a handle with a table but no address, a table with more
  entries than it records - and asserts that each is refused and released
  without touching the bytes it does not describe.
- `tests/additions.rs` exercises the operations that are not plain getters:
  entries, draining, retained mutation, spare capacity, duration arithmetic,
  wiping containers, and a guarded call that answers with a map, a box, an error
  and a duration.
- `tests/surface.rs` walks the accessors and the iterators: filling and replacing
  an optional value, wiping what a `take` left behind, turning a result around,
  borrowing and taking a box apart, iterating every collection in all three
  forms, splitting and joining maps, relating sets, taking a path apart, and
  adding lengths of time.
- `tests/collections.rs` and `tests/vectors.rs` cover the collections on their
  own: the ordered map and set, the hash map and set including the numbers they
  store, every sort and the block it takes, insertion and removal with the
  destructors that follow, and the editing helpers.
- `tests/enumeration.rs` declares enumerations and checks what the declaration
  writes: tags, payloads, the view, the wipe, the identifier, the traits a
  declaration names with `derives`, a payload a value cannot be built over, and
  the fallback a guarded table answers with.
- `tests/objects.rs` and `tests/guarded.rs` cover the generated tables: every
  receiver form and method shape, default bodies, an argument the receiving side
  refuses, a panic contained at the boundary, and the release of a receiver a
  consuming method took.
- `tests/rfc3339.rs` writes points in time as RFC 3339 text, reads them back,
  applies numeric offsets, and refuses the forms that name no valid instant.

## Running

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo run -p xt_kernel_native
cargo run -p xt_kernel_static
cargo run -p xt_kernel_app
```

`cargo test` runs one test at a time here: `.cargo/config.toml` sets
`RUST_TEST_THREADS = "1"`, because several test files keep their instruments -
counts of releases, of allocation calls, of shutdown hooks - in `static` values,
and two tests writing one counter read each other's work. A run that wants the
threads passes `-- --test-threads <n>`.

Without `std`, checked on a target that has no `std` to fall back on:

```sh
rustup target add wasm32-unknown-unknown
cargo check -p xt_stable_abi --no-default-features --target wasm32-unknown-unknown
```

`cargo test` alone does not build the dynamic libraries, so the end-to-end test
builds them itself when they are missing or older than the sources. The test
locates the profile directory from its own path, so `CARGO_TARGET_DIR` and
`--release` work.

The interpreter cannot load a dynamic library, so `tests/end_to_end.rs` is
compiled out under Miri. The rest of the suite runs:

```sh
MIRIFLAGS=-Zmiri-ignore-leaks cargo +nightly miri test -- --test-threads=1
```

`-Zmiri-ignore-leaks` is needed for the targets that keep allocator counters in
`'static` values and for the ones that build tables, because a table cache is
process-wide by design. The targets that do neither run with leak checking on:

```sh
cargo +nightly miri test --test checked_raw --test editing --test handles \
    --test hostile --test layout --test primitives --test status --test tuples_arrays \
    --test types --test unwind --test versions -- --test-threads=1
```

## Scope

- **`validate` checks a value, it does not sandbox a hostile producer.** It
  refuses what the value's own numbers contradict: a length past its capacity, a
  tag the type does not define, an address that is not aligned, a fraction at or
  past one second. It cannot know how large the block behind an address really
  is, so a header that reports `length == capacity == 1_000_000` over a
  one-byte block passes every check, and the read or the release that follows
  reaches past it. `load` is `unsafe` for the same reason: load only a library
  whose code you already trust. A protocol that has to survive a hostile library
  must hand the receiver a budget - the number of bytes it may read - and check
  every value against it before reading. This crate takes that budget in four
  constructors, [`StableStr::from_raw_parts_checked`],
  [`StableSlice::from_raw_parts_checked`],
  [`StableSliceMut::from_raw_parts_checked`] and
  [`StableBox::from_raw_parts_checked`]; each compares it at construction, while
  the accessors and the other constructors trust the numbers in the value.
- The descriptor carries one object. A library that publishes several services
  either exports several entry points or hands the extra objects out through the
  first one; `plugins/abi_plugin` does the second and `tests/end_to_end.rs`
  asserts it across the dynamic boundary.
- The host offers an allocator, a log sink and a query function for anything
  else. `plugins/abi_plugin` asks for a host service by identifier, and the test
  observes the host-side counter.
- A generic method and an associated item are reported at the declaration. One
  fixed slot cannot carry them; the diagnostic names the shapes that can, and
  "Method shapes" shows the one-method-per-instantiation form. A return type
  that names a lifetime and `-> Self` are refused at the declaration as well.
- `StableMap` and `StableSet` are sorted by key and find one with a binary
  search; [`StableHashMap`] and [`StableHashSet`] keep the caller's hash beside
  each entry and order by `(hash, key)`, so they find a key with a binary search
  and still keep the crate's rule that equal contents occupy equal bytes,
  whatever order the entries were inserted in. The hash function is an argument,
  so the crate carries no hasher and takes no random seed: the same entries
  produce the same value in every process. An entry is identified by
  `(hash, key)`, so a map is read with the function that filled it, and
  [`StableHashMap::validate_with`] checks a received value against the function
  it will be read with.
- Times are written and read as RFC 3339 in UTC or with a numeric offset
  ([`StableSystemTime::to_rfc3339`], [`StableSystemTime::to_rfc3339_with_offset`],
  [`StableSystemTime::from_rfc3339`]). Converting to a local zone needs that
  zone's rules for the date in question, which come from a database this crate
  does not carry; the side that has the database looks the offset up and passes
  it, and the instant is written in that zone.
- `wipe` is available through the [`Wipe`] trait for the plain-data types and
  for the containers that can wipe their elements: strings, vectors (including
  vectors of strings), maps, sets, boxes, arrays, tagged values, copy-on-write
  values and errors. An implementation overwrites the content and leaves the
  value valid, so the value is still released through its allocator afterwards.
  Operations that shorten a value (`truncate`, `remove`, `clear`, `take`) leave
  the bytes they give up in place; overwriting them is the explicit step.
- `Debug` on the owning containers reports the shape, not the contents, because
  the contents may be data a log must not repeat. Use `as_slice`/`iter` to read
  them.
- `unwind::guarded` runs the process panic hook, so a contained panic is also
  written to standard error as well as returned as an [`AbiError`]. A host that
  must not write there installs its own hook; the crate does not, because
  replacing a process-wide hook is the host's decision. `AbiError::wipe`
  overwrites a message that should not outlive the call.
- The crate does not open dynamic libraries and does not keep a registry of
  plugins. It provides the declarations, the checks and the keep-alive slot that
  such a host needs; which libraries to load and when to unload them is the
  host's decision.
- The crate builds without `std` with `default-features = false`. The `std`
  feature is on by default and gates `StableOsStr`, `StableOsString`,
  `StablePath`, `StablePathBuf`, `StableSystemTime`, `StableInstant`, the
  `std::error::Error` implementations and `stable_object_guarded!`, whose whole
  job is to contain a panic. Everything else, including `stable_object!`,
  `StableVec`, `StableString`, `StableMap`, `StableSet` and the entry protocol,
  is available either way. The allocation crate is imported as `heap` because
  this crate has a module named `alloc`, which is the allocator handle the
  descriptor carries.
- The oldest toolchain this tree is verified with is 1.90.0 (also built with
  1.97.1), which is what `rust-version` states.

## Documentation

| Document | English | 中文 |
| --- | --- | --- |
| Using and publishing a boundary library | [`docs/boundary-guidance.md`](docs/boundary-guidance.md) | [`docs/boundary-guidance.zh-CN.md`](docs/boundary-guidance.zh-CN.md) |
| Dependencies and scenarios | [`demo/README.md`](demo/README.md) | [`demo/README.zh-CN.md`](demo/README.zh-CN.md) |
| This file | `README.md` | [`README.zh-CN.md`](README.zh-CN.md) |

## License

See [`LICENSE`](LICENSE).

The repository is public, but the terms are not an open-source licence: use
outside the XT Engine has to be non-profit, and redistribution, redistribution
of a modified form and publication to a registry are not permitted.
`Cargo.toml` names the file with `license-file` and states `publish = false`
for the same reason.

Reading is not restricted: an implementation written from what the repository
teaches is the writer's own work, and a copy kept to oneself is not
distributed.

`repository` is not stated yet: the URL is not decided.
