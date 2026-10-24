# Using and publishing a boundary library

[中文](boundary-guidance.zh-CN.md) · **English**

This document is for people who use this crate: what a boundary library does
under two real pressures - **very dense calls** and **being reverse engineered** -
and what to do about it. Section zero is the hard rule about what may be
published, and anyone who hands an implementation to somebody else should read it
first.

Every number here names where it came from. Where something was not measured,
this document says so.

---

## 0. What may be published (read this section first)

One of the reasons this crate exists is to **deliver the implementation as a
binary, without handing over the source**. That reason holds only if the
deliverable is chosen correctly. What follows is a rule, not advice.

### 0.1 Three rules

1. **Ship the dynamic library only**: `.dll` on Windows, `.so` on Linux,
   `.dylib` on macOS.
2. **Never ship `rlib`, `.rmeta`, `.pdb` or `.d`.**
3. **Select files by name when packaging, never by directory.** Do not
   `Compress-Archive` all of `target/release`.

### 0.2 Why an `rlib` is the same as handing over the source

An `rlib` contains the `.rmeta` that cross-crate inlining and rustdoc read, and
that file holds **source-level information**, not machine code. Measured, using
four doc-comment sentences from this repository as probes:

| Artifact | Doc-comment hits | Size |
| --- | --- | --- |
| `xt_kernel_dynlib.dll` (the deliverable) | **0 / 4** | 275,968 B |
| `libxt_kernel_impl-*.rlib` (the implementation) | 1 / 4 | 278,726 B |
| `libxt_kernel-*.rlib` (the kernel) | 2 / 4 | 785,382 B |

Whole sentences of a crate's documentation can be read straight out of an `rlib`:

```
The map is the working form; [`TermStats::top`] produces the ordered form a
This crate is an ordinary Rust library: it knows nothing about stable
layouts, dynamic libraries or the ABI crate in this workspace. It exists to
```

That text is far easier to read than a decompiler's output - it was written for a
person in the first place. Decompiling costs effort; `strings` does not.

### 0.3 What happens to each file

| File | May be published | Note |
| --- | --- | --- |
| `xt_kernel_dynlib.dll` / `.so` / `.dylib` | **yes** | the only file that has to be published |
| `xt_kernel_dynlib.dll.lib` / `.exp` (MSVC import library) | yes, if needed | holds two export names only, `xt_kernel_entry` and `xt_kernel_shutdowns`; used when a C or C++ host links against the library |
| `*.pdb` | **no** | measured at 1,249,280 B, and it holds every Rust symbol name (`KernelWorkspace`, `top_terms` and so on) |
| `*.rlib` | **no** | see 0.2 |
| `*.rmeta` | **no** | the carrier of the doc comments |
| `*.d` | no | the build-time dependency list |

### 0.4 The mistake people make: archiving `target/`

After one `cargo build --release --workspace` from a clean directory, this
repository's `target/release/` holds **12 `.rlib` files (6.47 MB in total), 6
`.rmeta` files, 12 `.pdb` files and 24 `.d` files**. They are not incidental
by-products: they are the compiled output of each crate, including the
implementation that is meant to stay private.

So rule 3 is not fussiness: **packaging a directory ships the private parts with
the rest**, and none of the file names involved says "source".

In the same build, the deliverable crate produces no `.rlib` at all (measured: 0).
That is what the split in 0.5 buys.

### 0.5 Structural point: a crate whose `crate-type` includes `rlib` cannot be the deliverable

In an early version of this repository, `demo/dynlib` was
`crate-type = ["cdylib", "rlib"]`, because one of the comparison programs linked
it. That is exactly the shape to avoid: a deliverable crate that also produces an
rlib ships its own `.rmeta` if the build directory is ever packaged by mistake.

The repository is split the following way now, and the shape can be copied
directly:

| crate | produces | role |
| --- | --- | --- |
| `demo/impl` (`xt_kernel_impl`) | `rlib` | the implementation itself, for those who want the linked form |
| `demo/dynlib` (`xt_kernel_dynlib`) | **only** `cdylib` | the deliverable: exports two names and contains no code of its own |

Conclusions:

- **The deliverable must be the crate that produces a `cdylib` and nothing
  else.** If the implementation and the exports live in one crate, split it in
  two: the implementation goes into an rlib crate, and the deliverable crate only
  forwards and declares `crate-type = ["cdylib"]`.
- **`cargo publish` is not an option.** Publishing a crate is publishing its
  source. Every crate under `demo/` is `publish = false`, and
  `cargo publish --dry-run -p xt_kernel` returns 101 with
  `` `xt_kernel` cannot be published ``.

### 0.6 One command to verify before publishing

Run it on the directory you are about to package; the expected output is **zero
lines**:

```powershell
Get-ChildItem -Recurse <bundle> -Include *.rlib,*.rmeta,*.pdb,*.d | Select-Object FullName
```

On Linux and macOS:

```sh
find <bundle> -name '*.rlib' -o -name '*.rmeta' -o -name '*.pdb' -o -name '*.d'
```

The command was run against this repository: pointed at `target/release` after a
`cargo build --release --workspace`, it returns **54 lines**; pointed at a
directory holding only the `.dll`, it returns **zero**. So it catches the mistake
and does not fire on a correct deliverable.

Then confirm that the bundle holds the dynamic library and nothing else (plus the
optional import library):

```powershell
Get-ChildItem -Recurse <bundle> | Select-Object Name
```

### 0.7 For anyone changing this repository (automated agents included)

- Do not add `rlib` to the deliverable crate's `crate-type` just to make some
  program link. If you need the linked form for a comparison, add a separate rlib
  crate - `demo/impl` is that crate here.
- `demo/dynlib` declares `crate-type = ["cdylib"]` only; adding `rlib` gives the
  deliverable a copy of its own source-level metadata.
- Do not run `cargo package` or `cargo publish` against `demo/kernel`,
  `demo/impl`, `demo/dynlib` or any crate that contains an implementation.
- Do not put `target/` or `tmp/` into an archive, an image or an artifact store.
- Do not commit or publish a `.pdb`.
- When you have to prove that the deliverable carries no source-level
  information, rerun the probe from 0.2. Do not go by memory.

### 0.8 How a host integrates (this decides whether source has to be handed over)

There are two ways to integrate the same library, and only one is compatible with
keeping the code private:

| Integration | What the host needs | Compatible with secrecy |
| --- | --- | --- |
| Loading at run time (`xt_kernel_api::loader::Plugin::load`) | a `.dll` / `.so` / `.dylib` | **yes** |
| Linking (`extern crate`, an rlib in the dependencies) | an `rlib` | **no**: an rlib is the implementation |

So when a host asks for "a library I can link directly", the request itself is a
request for the source. The correct answer is: declare the interface with
`xt_kernel_api`, and load the implementation at run time with `Plugin::load`. If
the host also wants to build an executable of its own, it needs the API crate and
not the implementation crate.

`demo/app` in this repository is that integration; `demo/static` is the
exception, and it links the rlib only so that "the same protocol" can be compared
side by side. It is not the recommended delivery shape.

### 0.9 Unloading at run time (swapping mods, hot reload)

Dropping libraries and loading others without ending the process is possible, but
**the host guarantees the order**. Dropping a `Plugin` does three things in
sequence: releases the objects the library handed to the host, runs the library's
own shutdown hook, and only then calls `FreeLibrary`/`dlclose`. The host has four
obligations of its own:

1. **Every owned value the library allocated** must be dropped before the unload
   - not only the root object and the sessions and child objects reached through
   it, but every owned value any call **returned**: `StableString`, `StableVec`,
   `StableBox`, `AbiError`, `StableMap`… each one carries the library's own
   release function pointer, and dropping it calls that function. Anything the
   host received from the library has to die before the unload point.
2. Threads the library started must be joined in its shutdown hook;
   `FreeLibrary` does not wait for threads.
3. Callbacks you registered with the library have to be removed before the
   unload.
4. Nothing synchronises the unload against a call that is entering the library.
   That is the host's responsibility.

Why the first one is easy to miss: `LoadedPlugin` releases only the few fields in
the descriptor (`name`, `api`). **It cannot track** the values you received from
methods - their ownership is already yours, and the host is the only one that
knows where they are. So the owned values in host state and the `LoadedPlugin`
need a defined drop order, with the former dying first:

```rust
// The host's own layer, holding everything it received from the library.
struct HostState {
    names: Vec<StableString>,
    last_error: Option<AbiError>,
    session: Option<LoadedPlugin>,
}

impl Drop for HostState {
    fn drop(&mut self) {
        // These carry the library's release functions, so they go before the
        // session.
        self.names.clear();
        self.last_error = None;
        // The session drops last: it runs the shutdown hook, then unloads the
        // library.
    }
}
```

Put another way: **do not keep an owned value the library returned anywhere that
outlives the `LoadedPlugin`** - a process-wide `static`, another thread's queue,
or a variable declared before it in the same scope all do that. Within one scope
`drop` runs in reverse declaration order, so if you skip the wrapper above you
have to get that order right yourself.

On a lease/guard mechanism that would make it impossible for the host to carry a
library-allocated value past the unload point: **this crate does not add one.**
The set of such values is **open** - any `AbiType` can be a return type - so a
guard that could hold them would need a wrapper generated per type, which amounts
to a second layer of ABI types on top of the boundary types; and what it would
block is still only the writing "the host stores the value somewhere" (once a
value is split into fields and put into a container of the host's, the guard
cannot see it). A documented contract, a drop order and the wrapper above are the
form this constraint can take. Truly mechanical enforcement leaves one road:
process isolation.

One more item is **the library's own business**: a virtual table is built once per
implementation type and shared by every value of that type, so it outlives all of
them, its memory comes from the process allocator, and once the library is gone
nothing can release it. A library should call `release_cached_tables()` from its
shutdown hook (the hooks in `demo/impl` and `plugins/abi_plugin` both do).

Measured (this machine, the ignored test in `demo/api/tests/unload.rs`): without
that call, each load and unload cycle leaves about 480 B (2000 cycles, private
bytes +958,464); with the call, measured over 10,000 cycles in batches of 2000,
the net change is zero (one batch showed a 20 KB step that the system then took
back; the resolution of the measurement is one 4 KB page). The same test also
asserts with `GetModuleHandleW` that the module is mapped before the drop and not
mapped after it.

### 0.10 `validate` checks that a value agrees with itself, not that the producer can be trusted

`AbiType::validate` refuses, before safe code reads a value, the case where **the
value contradicts itself**: a length past the capacity, a tag the type does not
define, an address that is not aligned, a fraction at or past one second, slots
that disagree with the declaration. It cannot refuse the other case: **the block
behind the address is not that large**.

```
length = capacity = 1_000_000, pointer -> a one-byte block
```

Every one of those header fields passes (the length is inside the capacity, and
the byte count is addressable), and then `validate` reads element by element and
`Drop` releases 1 MB. The checker does not have the information "how large is this
block really" - `StableStr` and `StableSlice` do not even carry a capacity field.

So the threat model has to say it plainly: **`load` is `unsafe`, and it requires
the code of the library you load to be trustworthy**; `validate` is there to
catch "the same declaration, on two sides, drifted", not to keep a hostile plugin
out. For a receiver to survive a hostile producer, the protocol has to pass in
**the number of bytes the receiver actually holds**: this crate has four
constructors that take that parameter (`available`) -
`StableStr::from_raw_parts_checked`, `StableSlice::from_raw_parts_checked`,
`StableSliceMut::from_raw_parts_checked` and
`StableBox::from_raw_parts_checked`; they compare it **at construction**, and the
accessors and the constructors of `StableVec`/`StableString` trust the numbers in
the value. That is protocol design, not a layer the checker can add.

The other direction is the same point: **the description the host hands the
library has a version and a layout too.** `HostServices::size` says how many bytes
are covered, `version` says what the protocol means, and `layout_revision` says
where each field sits; a library should look at all three.
`plugins/abi_plugin` does. Comparing only `size` and the major number misses the
case "same major, same size, but the fields were reordered": there `size` passes,
the major number passes, and the library reads `allocator`/`log`/`query` at the
wrong offsets and then calls through whatever function pointer it read.

---

## 1. Very dense boundary calls: a million objects

### 1.1 What one crossing costs

The same workload, the same machine, a release build, three paths run three times
each, and the table takes the smallest value per scenario (linked in =
`demo/static`, loaded at run time = `demo/app`, reference = `demo/native`):

| Scenario | Direct call | Boundary (linked in) | Boundary (loaded at run time) |
| --- | --- | --- | --- |
| `split_fixed`, one number in, two out, **no allocation** | 0.5 ns | 6.7 ns | 6.7 ns |
| `classify`, one number in, one string out | 21.1 ns | 60.4 ns | 60.0 ns |
| `analyze` (cache hit), one string out | 117.3 ns | 151.3 ns | 156.5 ns |
| `top_terms`, one sort, two pairs out | 425.8 ns | 564.3 ns | 603.9 ns |
| `tokens`, five strings out | 248.6 ns | 565.1 ns | 568.8 ns |
| `reachable`, two edges in, one sequence out | 278.4 ns | 392.1 ns | 405.6 ns |
| `digest`, 4 KiB in, one string out | 9,142.9 ns | 9,260.9 ns | 9,113.8 ns |
| `session` round trip (four calls) | 277.0 ns | 373.5 ns | 437.0 ns |

Conditions: Windows x86_64-msvc, the default release profile,
`xt_kernel_dynlib.dll` at 275,968 bytes (the build these numbers were measured
against). The same configuration built on 2026-10-24 comes to 293,888 bytes; the
difference is the container methods, derived implementations and stable hashing
added afterwards, and none of them is on these call paths. Each scenario is warmed
up, then timed as a batch, the batch mean is taken, and the smallest of three
batches is reported. Owned return values are released at the end of each round, so
the numbers include the release, as real use does.

#### First thing to be clear about: linking in is not the same as having no boundary

"Statically linked" here only means **the implementation is linked into the
executable**; it does not mean the call became a direct call. The three paths do
different things:

| Path | Call form | Return value |
| --- | --- | --- |
| `xt_kernel_native` | a direct call to `Engine::split_fixed` | a Rust value |
| `xt_kernel_static` / `xt_kernel_app` | a call through a function pointer in the object's table | a boundary type, built with the host's allocator |

The last two paths **run the same protocol**: look up a slot, call indirectly,
build the answer as an owned boundary value, validate it. Loading at run time adds
one one-off step - open the file, look up two symbols. So the per-call cost of the
two has to be identical, and the table shows that it is. **Linking in saves
deployment and start-up, not per-call latency.**

#### Second thing to be clear about: the money goes into building the return value

The `split_fixed` row exists as the control: it does arithmetic and allocates
nothing. Its boundary cost is **6.7 ns**, while the direct call is essentially
zero. So:

- **The crossing itself: <= 6.7 ns.** That is the whole of "look up a slot + call
  indirectly + validate".
- **Each returned owned value: about 30 to 40 ns.** `classify` is 60.4 - 21.1 =
  **+39 ns**, and `analyze` is **+34 ns**. The reason is one extra allocation and
  one extra copy: the kernel produces a `String` of its own, and the crossing has
  to build a second, owned boundary string through the host's allocator, which is
  then released at the end.
- **Each returned string costs more in bulk.** `tokens` is +317 ns for five
  strings, about 63 ns each (including container growth and five paired
  allocation/release steps).
- **Input is charged by byte, not by element.** Every argument is read once at the
  boundary: a string is checked for UTF-8, a container for its header and then
  every element. The cost is linear in bytes. Measured here (`tmp/validate_probe`,
  release, smallest of three batches) for a `StableStr` argument: 1 KiB 21.1 ns,
  4 KiB 56.0 ns, 64 KiB 983.7 ns, 1 MiB 16.5 us, about 15 ns/KiB. So of the
  +118 ns on `digest`, the check of the 4 KiB input (about 56 ns) is roughly half;
  the rest is building the return value and noise. The two edges in `reachable`
  are tiny, so its +114 ns is essentially not validation.
- **Several accessors in one call add up.** The `session` round trip is four
  calls: +97 to +160 ns.

So the cost model is: **about 7 ns of fixed cost per call + 30 to 40 ns per
returned owned value + validation charged by byte (about 15 ns/KiB)**. It is
**not** a charge per input element - and that difference decides the advice
below: sending data in bulk is cheap (about 16 us per MiB), returning many small
containers is expensive.

One measurement caveat: feeding `split_fixed` constant arguments lets the
optimiser hoist the direct call out of the loop entirely, which measures 0.2 ns of
nonsense. The numbers in the table come from arguments that change every round.
Any "boundary vs direct" comparison has to confirm first that the direct path was
not optimised away.

#### Slot lookup does not grow linearly with the number of methods

The entries are ordered by method identifier when the table is built, so a table
with more than 16 entries finds a slot by binary search. Up to 16 entries both
kinds of table are walked linearly (the loop overhead of divide and conquer is
more expensive in that range), so the first three rows measure where the entry
happens to sit, not a difference between the algorithms. A hand-written,
unordered table is never searched by bisection: it is walked from the front. A
miss in an ordered table answers "not present" directly instead of being followed
by another scan.

Measured here (release, smallest of three batches, the last lookup in one table,
which is the worst position for a binary search):

| Table length | Ordered table (bisection above 16) | Unordered table (one walk) |
| --- | --- | --- |
| 4 | 1.64 ns | 0.73 ns |
| 8 | 2.27 ns | 1.46 ns |
| 16 | 3.73 ns | 2.91 ns |
| 64 | 5.55 ns | 12.67 ns |
| 256 | 7.39 ns | 52.95 ns |
| 1024 | 9.34 ns | 193.82 ns |

So for the common small trait (a dozen methods) the two are within 1 ns of each
other; the ordered form starts to win at 24 entries, is more than twice as fast at
64, and about 20 times faster at 1024. The byte layout of the table is unchanged
(header, entries, sentinel); only the order of the entries differs, and the order
is not part of the protocol: a call finds its entry by identifier, not by
position.

### 1.2 Why "one crossing per object" cannot be saved by optimisation

Using the measured numbers from the table above rather than a pessimistic
constant:

```
Crossing only, no owned return value:  10^6 x 6.7 ns = 6.7 ms    (a 60 fps frame is 16.7 ms)
One string returned per call:          10^6 x 60  ns = 60  ms    (3.6x over already)
```

6.7 ms says that **"one crossing per call" on its own is not out of the running**:
it takes 40% of the frame budget, which is ugly but not impossible. What kills it
is the next two lines:

```
One owned value returned per call:     another 10^6 x 39 ns = 39 ms
The data itself:                       10^6 x 64 B = 64 MB per frame, about 3.8 GB/s at 60 Hz
```

So the question is not whether the boundary is fast enough. It is these two:

1. **Return values must not be built per object.** A call that returns N owned
   values pays N allocations and N copies. That is removable (batching, plus a
   host arena).
2. **The data has a physical ceiling.** 3.8 GB/s is memory-bandwidth territory;
   even a free boundary would not remove it.

Conclusion: performance here is decided by **the shape of the calls**, and the
room in per-call overhead is a few nanoseconds.

### 1.3 Which layer owns what

| Layer | Owns | Does not own |
| --- | --- | --- |
| This crate (the ABI layer) | making the batch **expressible**: borrowed slices in with no copy, host-supplied allocator taking over the out direction, stable shapes | scheduling, thread pools, SIMD - those are algorithmic and policy decisions |
| Your API facade | folding "once per object" into "once per batch"; passing the arena allocator down | deciding how parallel the kernel is |
| Your kernel | batch algorithms, SoA layout, SIMD, parallelism | specialising data structures for the boundary |
| The host program | reusing buffers, choosing the allocator, controlling call frequency | expecting the ABI layer to batch on its behalf |

### 1.4 Suggested order (by payoff)

**1. Batch.** Replace N calls with one. Pass input as
`StableSlice::from_slice` - it **borrows the caller's memory**, with no copy and
no allocation, so `transform(StableSlice<Mat4>)` is writable today. Your matrix
type only has to be `#[repr(C)]` and declared once with `unsafe_stable_element!`
(the `unsafe` in the name is that promise: same layout and offsets on both
sides).

**2. Let the host supply the allocator.** Containers returned from a call use the
allocator **the host passed in**, not the library's own:

```rust
let arena = AbiAllocator::from_parts(arena_ptr, allocate, deallocate, reallocate);
let host = HostServices::new(arena);
```

Every allocation inside the library then becomes a bump in the arena, **reclaimed
as a whole at the end of the frame**, with no `free` between calls. That path
exists already; what is missing is an example. If you are doing this, write it in
that shape first rather than changing the crate.

**3. Reuse objects; do not rebuild them every frame.** Opening a session, parsing
the descriptor and adopting the object happen once, at start-up. The object
`Plugin::load` returns should live for the whole run.

**4. Let hot paths return scalars or handles, not small containers.** Each
returned owned value costs about 30 to 40 ns (one extra allocation and one copy),
while the crossing itself is 7 ns. When you need many results, return the whole
sequence once rather than one value N times.

**5. Move the loop into the library.** If an operation is inherently "ask once per
object", let the library loop: the host crosses once and the library iterates N
times inside. The boundary is good for moving data; it is bad for moving control
rhythm.

**6. Do not treat "linked in" as a performance measure.** What it saves is
deployment and start-up (one file, one symbol lookup), not per-call cost. To get
per-call cost near native you need 1, 2 or 5.

### 1.5 Known limits (measured against the current implementation)

Stated plainly so that you do not design against a shape that does not exist:

- **A writable borrow is a borrow for one call.** `StableSliceMut<'a, T>` lets an
  implementation modify your buffer in place (the test
  `a_method_can_write_through_the_storage_it_was_given` in `tests/objects.rs`
  takes that road). But `'a` comes from the caller, and the compiler will not let
  an implementation store the handle in one of its own fields: the handle is
  valid for that call only.
- **A boundary value that owns storage is not `Send`/`Sync`.** It carries the
  allocator functions and context that built it, and the crate cannot prove the
  other side accepts calls from another thread. The correct shape for multiple
  threads is **one handle per thread** (or one load per thread); do not pass one
  object across threads. Values with no allocator (the borrowed views `StableStr`,
  `StableSlice`, `StableSliceMut`, `StableOsStr`, `StablePath`, plus
  `StableArray`, the tuples, `StableDuration`, `AbiVersion`, `Handle` and the time
  values) are decided by the compiler from their members: they usually are
  `Send + Sync`, because they carry only their own parts. **The copy-on-write
  values are a counterexample to that rule**: both fields of `StableCowStr` /
  `StableCowSlice` exist in the layout, and the inactive one is still a whole
  `StableString` / `StableVec`, so even the `borrowed` form is not `Send` - that
  is the price of a fixed layout, not an oversight; to cross a thread, call
  `into_owned` on that side and then treat it as an owned value.
- **Slot lookup costs something, but it can be amortised.** A generated table is
  ordered by identifier when it is built, and tables above 16 entries are searched
  by bisection; tables of 16 or fewer, and hand-written unordered tables, are
  walked linearly. The cost is negligible inside one call, so batch first instead
  of starting with the lookup.
- **Every return value is validated in full** before safe code sees it: a string
  over its whole byte range for UTF-8, a container element by element, an object
  for its table header and address. So a batch return **walks the results again**;
  that cost is separate from the fixed per-call cost, and it only shows on large
  return values. When the element type has no content to check (`u8`, `u64` and
  the like), the element loop is optimised away entirely in release: measured
  `StableSlice<u64>` `validate` takes about 4.3 to 4.4 ns from 1 element to
  1,048,576 elements, independent of length. So "charged by byte" holds for
  strings and not for slices of plain numbers.
- **Arguments are validated in full too, only on a different side.** The accessor
  a `stable_object!` table generates validates each argument **on the sending
  side**: a bad argument is abandoned and the call panics (that table has no status
  word to report through). A `stable_object_guarded!` table validates arguments
  **in its trampoline (the receiving side)**: a bad argument is refused with
  `AbiStatus::InvalidArgument` and the implementation never runs. So the cost of
  arguments has the same source as that of return values: use the guarded
  declaration when the two sides do not trust each other, and either way the
  pass over the arguments happens (only who runs it and how a failure is reported
  differ).
- **Every object handle, passed or received, is compared against the receiver's
  declaration.** `validate_handle` checks the table header and then compares the
  method sets. When the entry count matches the declaration and the combined
  header signature matches too, it still looks every slot signature of the
  declaration up in the table (the table is ordered, so that is a bisection; a
  hand-written unordered table is walked) - the combined signature is a 64-bit
  summary of the list, not the list. Only a real revision relationship ("the peer
  declares a few methods fewer" or "a few more") takes the item-by-item path, and
  that path is **one lookup per peer entry against the declaration**: for a trait
  with 1024 methods, with the two sides at different revisions, one handle pass
  costs about a million comparisons. A common trait has a handful of methods and
  this is negligible; but it happens on every handle pass, not only at adoption.
- **A sort's temporary buffer comes from the value's own allocator** (not the
  process allocator), so an arena's "reclaim at the end of the frame" holds for
  sorting as well; the price is that a failed sort returns an error rather than
  panicking (the fallible entry points) or panics (`sort`/`sort_by`). A stable
  sort inherently needs a temporary block the size of the vector (peak 2x). To
  avoid it, use `sort_unstable_by`: it is a heapsort that swaps inside the
  vector's own block, **allocates nothing**, and therefore has no fallible form;
  the price is that the relative order of equal elements is not preserved.
- **Object construction takes a process-wide lock only the first time a type is
  registered**, one table per trait. When an implementation type is registered for
  the first time, the table is built under the lock and an immutable copy of the
  entries is published; afterwards constructing objects of that type reads that
  copy without locking. So repeatedly constructing objects of one type on the hot
  path never waits, while several threads constructing the **first** object of one
  type do serialise once. The price is that **every publication copies all the
  entries and keeps the previous copy** (threads still reading the old copy have
  to be able to finish), so one trait retains `N(N+1)/2` entries (N = the number
  of implementation types registered in this process for that trait; each entry is
  24 bytes: a type identifier and a table address). N=100 is about 120 KB, N=1000
  about 12 MB. It grows with the **number of implementation types**, not with the
  number of constructions, and `release_cached_tables()` hands the whole chain
  back at unload; N is decided by the code in the process, so it is usually a
  one-off cost. Releasing old copies sooner would require every reader to announce
  "I am reading", which is a read-modify-write on a cache line every thread
  shares - exactly the cost this lock-free read exists to avoid. So the chain
  stays, with its shape written down.
- **The length of a container of zero-sized elements has no upper bound, so the
  boundary refuses one.** A `StableVec<()>` occupies no buffer, so its `length`
  cannot be derived from a capacity: a peer writes a header saying "length 2^63"
  and every header check passes, while the receiving side's `validate` would walk
  that many elements. The `validate` of `StableVec`/`StableSlice`/`StableSliceMut`/
  `StableMap`/`StableSet` therefore refuses "zero-sized element + non-zero
  length": such a container carries nothing but that number, and no byte count,
  alignment or address can show that it is real - and this crate will not invent
  an upper bound on the caller's behalf (any concrete number would be a guess, not
  a property of the protocol). The header check `validate_header` still accepts it,
  because the same function also decides whether this process may release the
  value, and the elements of a locally built zero-sized container still have to be
  dropped. If a sender has only one thing to say, send that number as a number. If
  a receiving side wants a budget of its own at its entry point, note that
  `available` in `from_raw_parts_checked` applies to construction, not to the
  accessors.
- **A hash container only works with the same hash function on both sides.** An
  entry is identified by `(hash, key)`; `validate` only checks that the pair is
  strictly ascending and that the elements are valid, and **does not recompute the
  hash** - the receiving side does not hold the function. So when the sender uses
  A and the receiver uses B, the value passes `AbiType::validate` and then every
  `get` misses. Two ways out: (1) the caller supplies the function
  (`insert(hash, ...)`), and a receiver that knows which function to use checks
  with `validate_with` before reading; (2) the crate supplies it - `StableHash`
  fixes the numbering inside this crate (FNV-1a over an encoding that names width
  and byte order; `usize`/`isize` are **not** included because their width follows
  the target), so a map built with `insert_stable` can be checked on its own with
  `validate_stable` and the two sides need agree on nothing in advance. Putting an
  **identifier** of the hash function into the value does not solve it: the two
  sides still have to agree on "the same function", and storing one more number
  only moves that agreement somewhere else; the number the crate computes can be
  checked precisely because it does not depend on either side.

### 1.6 Measuring it yourself

The three programs in `demo/` are the instrument: the same scenarios, the same
expected answers, three ways to arrive. After changing the kernel, run all three
and judge boundary cost by the **differences**, not the absolute numbers. Absolute
numbers drift with the machine, the compiler version and the optimisation level.

---

## 2. How much code a reverse engineer can recover from a dynamic library

### 2.1 The ceiling, stated first

Published native code is information. Disassembly gives control flow, constants
and data layout; a decompiler turns that back into readable structure. **No
measure keeps the algorithm in a binary from being disclosed**; it only raises the
cost of reading it. So the meaningful question is not "can it be hidden" but "are
we giving something away for free".

What follows is **measured in this repository**, not inferred.

### 2.2 Once given away, now fixed: the complete method signature table

The `.rdata` of the release DLL before the fix held plain text:

```
analyze (shared guarded), StableStr<'_> -> StableString;tokens (shared guarded), ...
... open_session (shared guarded), StableStr<'_> -> SessionObject;add (mutable guarded), ...
```

Every method's name, argument types, return type and receiver kind was in there -
an interface map for free.

The cause was not the design but the implementation: the signature hash was
computed when the table was built, at run time, from
`declaration_id(stringify!(...))`, so the text had to exist as data. The table
only needs the `u64`; the text was the raw material for the hash.

That computation now happens at compile time, and identifiers no longer come from
text at all: every boundary type has a number (`AbiIdentity::IDENTIFIER`), and a
method's identifier is folded from the numbers of the types it mentions. So the
**signature** has no text left to recover, at any optimisation level.

**That statement covers "no plain text" only.** The fold is unsalted, and the fold
algorithm is a public protocol (a hand-written table has to implement it), so
someone with the binary can enumerate candidate method names and argument type
combinations, compute `slot_id` and compare it against the eight bytes in an
entry; the same holds for trait names (`declaration_id(name)` is plain FNV). That
is not "executable code" but "the interface map can be reconstructed offline", at
the cost of an enumeration. Blocking it needs a salt the two sides share, which
requires building both from one source, or an opaque identifier supplied by the
caller (`abi_try_from_value_with` already takes that parameter; one implementation
type can only be registered under one identifier in a process, and a second one
panics). This crate does neither, because there is no shared secret to put in.

Measured in the round that fixed it (both sizes from the same configuration,
before and after):

| | Before | After that round |
| --- | --- | --- |
| `xt_kernel_dynlib.dll` size | 278,528 B | 281,088 B |
| Signature text (`shared guarded`, `StableStr<`, `top_terms`) | visible | a per-string search returns false for each |

The size follows the binary's contents and is not a constant: the same
configuration built on 2026-10-24 is 293,888 bytes. The three demo paths all pass
in one build of that configuration (native 33 checks, static 37, dynamic 37, zero
failures), which shows that the slot identifiers the two sides compute around one
declaration still line up one for one. The numbers come from the declaration
itself: the same type answers one number whether it is written `StableStr`,
`StableStr<'_>` or with a full path, and an alias written another way lands on
that number too.

**The table's `type_id` is a different matter.** It comes from the trait's name
(`type_id_of(stringify!(Trait))`); the fold happens at compile time, but that
literal is still an input to the call. Measured: a release build of the same DLL
has no trait name in it, while **a debug build does** (`Child`, `Session` and
`Workspace` sit next to each other in `.rdata`). So "the trait name does not
appear in the binary" holds only for optimised builds; to remove it from debug
builds as well, give the trait a hand-written numeric identifier and stop letting
the name take part.

### 2.3 What still leaks (measured, still present)

| Content | Example | Source | Removable in stable Rust |
| --- | --- | --- | --- |
| File names and line numbers in panics | `demo\kernel\src\engine.rs` | where `panic!` is | no (nightly has `-Zlocation-detail=none`; not verified here) |
| Assertion and error text | `cannot be measured that way`, `the cache was not poisoned` | your implementation | yes, but you have to rewrite the text |
| Method names (for diagnostics) | `the object does not declare the method` | this crate's diagnostics | yes, at the price of worse error messages |
| Full symbol names | `KernelWorkspace`, `top_terms` | **the `.pdb` file** | yes: do not ship the PDB |
| Addresses of the allocator, log and query functions | the four pointers at the head of every owned value | the value contains its `AbiAllocator` | no, but they are addresses in this process, not code |

On the last row: **any owned value you hand out carries the addresses of your
allocator functions and context** (`StableString`, `StableVec`, `StableBox` and
`StableDyn` all do; the inactive `owned` field of `StableCowStr::borrowed` carries
one too). `Debug` deliberately does not print those addresses, but **the bytes on
the wire do carry them**. What they leak is this process's memory layout, not the
implementation; if that is unacceptable too, do not send owned values to a party
you do not trust - use borrowed views, which carry only a pointer and a length.

A guarded declaration **does not send the panic payload across the boundary**:
the trampoline writes a status word, and the caller reads `AbiStatus::Panic`. A
hand-written entry point that uses `unwind::guarded` copies up to 4096 bytes of
the panic message across, which is the decision of whoever writes that entry
point. One case deserves its own sentence: validating an argument means running
the argument type's own `validate`, and when that panics inside the trampoline the
implementation has not run and the receiver's value has not been moved out, so
that road reports `AbiStatus::InvalidArgument` and the handle releases its value as
usual.

Two measured conclusions worth remembering:

- **`strip = "symbols"` has no effect on this DLL for the MSVC target**: the size
  did not change at all (275,968 B -> 275,968 B) and the PDB is still produced.
  Do not treat it as protection.
- **`llvm-strip --strip-all` has no effect either**: this PE has no symbol table
  to strip, so the size is unchanged. What actually carries the symbol names out
  is **the `.pdb` beside it** (measured at 1,249,280 B). Do not ship it.

### 2.4 Obfuscation that does not break the ABI

The principle first: **a stable ABI constrains only what is on the boundary** - the
exported entry symbols, the descriptor protocol, the layout of the published
types, and the slot-hash algorithm. It **does not constrain the library's internal
implementation**. So most obfuscation is compatible with the ABI in principle, as
long as it does not touch those four things.

| Measure | Effect on the ABI | Effect on performance | Note |
| --- | --- | --- | --- |
| `lto = "fat"`, `codegen-units = 1` | none | not compared before and after in this repository | cross-crate inlining removes most function boundaries; the largest obstacle to a reverse engineer among these, paid for only in build time |
| no PDB, symbols kept out of band | none | none | the best payoff per unit of effort |
| plain-text panic and assertion messages removed | none | none | the text leaks module structure (see 2.3) |
| string encryption | none | some (decryption cost) | worth it for "key / licence decision" strings; do not turn it on globally |
| control-flow flattening, bogus control flow | none | 10% to 100%+ | needs an LLVM pass; enable it for **non-hot** modules only, and leave hot modules optimisable |
| per-customer builds (different seed / different trimming) | none | none | the ABI is unchanged, so the host builds once and swapping the library needs no rebuild - this is the **positive** value of a stable ABI for secrecy |
| packing, virtualisation | **high risk** | large | conflicts with run-time loading, memory attributes and signature checks; not recommended for a `cdylib` |

### 2.5 "No ABI change, no performance loss, still obfuscated" - how far it goes

Answered in three parts, because the three are coupled to different degrees:

**No ABI change: yes, and that is the normal case.** The first six rows above do
not touch the boundary. As long as the entry symbols, the descriptor protocol, the
layout of the published types and the slot-hash algorithm are unchanged, the host
does not change a line.

**No performance loss: true for the cheap tier only.** LTO, a single codegen unit,
no symbols, sanitised messages and per-customer builds do not add instructions to
hot paths (the before/after LTO difference was not measured here). They are also
**the most effective**: obfuscation works by losing the reader, and inlining plus
no symbols has already removed most readable structure.

**Heavy obfuscation with zero performance cost: no.** Control-flow flattening and
virtualisation add instructions to every path by construction; the two goals are
directly opposed. Secrecy costs money outside the hot paths.

One hard conflict has to be written down: **`panic = "abort"` is incompatible with
the guarded macros.** Containing a panic inside the library relies on
`catch_unwind`, which needs unwinding. With abort turned on, one panic inside the
library ends the host process and the containment semantics are gone. Do not turn
it on to make reverse engineering harder.

### 2.6 If the threat is "somebody changes it and calls it theirs"

This threat differs from the one above: it is not "somebody understands my
algorithm" but "somebody takes the whole code base, changes it a little and ships
it as their own product". The measures differ a lot, and mixing the two makes
people pick the wrong tool.

**Separate the two ways the code leaves first**:

| Way | What they have | Can a technical measure stop it |
| --- | --- | --- |
| A. They have the source (leak, outsourcing, an employee leaving, a mistaken publication, `cargo publish`) | all of your source and comments | **no**. Technology can only avoid giving it away, prove precedence, and trace who leaked it |
| B. They have the binary only | machine code | raise the cost; still cannot stop it |

The impersonation risk comes mostly from A. So the focus is not obfuscation but
**what is actually in the deliverable**. The hard rule and the measurements are in
section zero (0.2 to 0.6); the judgement to keep is: a `cdylib` has no doc
comments from this crate, an `rlib` does, therefore **ship the dynamic library
only**.

That rule explains the practical value of the four-layer structure in this
repository: **"published" and "private" are cut into different crates.**
`xt_kernel_api` can be open-sourced along with the host, `xt_kernel` appears only
as a binary, and a stable ABI sits between them.

#### Four things that work against A (they have the source)

1. **Ship no source, and no rlib either.** A private registry, or the binary
   directly; `cargo publish` publishes source, so do not use it to distribute the
   kernel.
2. **Shrink the source's exposure.** Repository permissions, CI logs and
   outsourcing scope stop at the API crate; do not copy an implementation into the
   examples or the benchmark (`demo/bench` holds only inputs and expected answers,
   no algorithm).
3. **Prove you had it first.** Sign the deliverable and register its hash (a
   third-party timestamp works). Legally this kind of evidence is worth more than
   any obfuscation.
4. **Contracts and licences.** Against "change it a little and call it mine" this
   is the only measure with direct effect: ownership, confidentiality terms,
   prohibitions on reverse engineering and redistribution. Technical measures make
   a breach detectable; they do not replace the contract.

#### Two things that work against B (they have the binary only)

1. **A per-customer build fingerprint.** Embed a build identifier (customer
   number, build time, source commit) in a **cold path** such as
   `capabilities()`. Zero cost on hot paths, no ABI change, because it is only
   string data. It answers "whose copy leaked" but **cannot** stop somebody from
   rebuilding.
2. **LTO plus a single codegen unit.** Push the decompiled output further from
   maintainable code. This is the only measure that costs no performance.

#### One honest boundary

**Once the source leaves your build machine, nothing technical stops "change it a
little".** Technology can do three things: not give it away, trace it, and prove
which came first. Any scheme claiming to "prevent copying" is selling a fourth.

If you need "it is useless even in their hands", there are only three roads: keep
the algorithm on a server, bind it to hardware, or accept "raise the cost" instead
of "prevent disclosure".

A stable ABI helps here: it lets you ship one implementation to different
customers, replace it wholesale at any time, and build per customer, without the
host rebuilding. **Trading replaceability for secrecy is more reliable than hiding
code.**

### 2.7 How far wiping reaches

`Wipe` covers **content**, not **description**:

- After a `StableString` is wiped, the byte block holding the characters is zeroed,
  but its own header (address, length, capacity, allocator) is still where it was -
  the value has to release that block after the wipe, and wiping the header would
  leak the block.
- A container's `wipe_in_place` wipes its elements and also **the slots no longer
  inside the length** (the content `truncate`/`clear` left in the block). The hash
  map and set go through the same code, so spare capacity is covered there too;
  each live entry also zeroes **the 64-bit hash stored with it**, because that
  number is a function of the key and keeping it keeps a fingerprint of the key.
- After `StableOption::take` moves a value out, its bytes are still in the slot
  (the same policy as `truncate`); `wipe_in_place` then zeroes the payload storage
  directly, so "take, then wipe" removes the residue.
- After a `StableDyn` is wiped, the block it occupied is zeroed, but **the nested
  allocations its destructor released are not wiped**: if the implementation holds
  a `String`/`Vec`, those blocks were handed back to the allocator inside its own
  destructor, this crate never sees their addresses, and cannot write to them. To
  wipe those as well, the implementation has to wipe them in its destructor (or
  implement `Wipe` and have the owner call it first).
- What remains in a released block is "there was a block, this long, allocated by
  this one" - no content.

So for data such as keys and tokens, wiping works; for "what the whole value
looked like", only the side that owns the storage (and knows where the block is)
can deal with it at release time.

Two more places leave fingerprints without content, and you have to decide whether
that is acceptable:

- `Debug` prints `type_id` and `signature` for a table header and for an object;
  they are FNV hashes of the trait name and the method list. The names themselves
  do not appear, but a fingerprint is enough to compare a dictionary of candidate
  names against. If a log is going to a party you do not trust, do not print those
  two values.
- `AbiError::Display` prints the message text. The example library in this
  repository now sends only the **kind** of failure across the boundary
  (`the graph has a cycle`) and no longer sends the kernel's own sentences
  (including counts and offsets) - errors are the kind of value that ends up in a
  log. A host that writes `Display` straight into its own log carries arbitrary
  text written by the library out of the process; to control that path, forward
  only `status()` at the boundary.

---

## 3. Two checklists

### Dense calls

- [ ] Are there still boundary calls that happen "once per object" on hot paths?
- [ ] Is input borrowed directly as a `StableSlice` instead of being copied into a
      `StableVec` first?
- [ ] Do return values go through the arena allocator the host supplies? (about 30
      to 40 ns per owned return value)
- [ ] Are handles and sessions created at start-up and reused, rather than rebuilt
      every frame?
- [ ] Are hot paths returning many small containers? (about 30 to 40 ns per owned
      value, while the crossing itself is 7 ns)
- [ ] Is one boundary value being passed across threads? (not allowed: one handle
      per thread)
- [ ] Was the **difference** remeasured with the three `demo` paths?
- [ ] In the comparison, was the direct path optimised out of the loop? (constant
      arguments produce nonsense numbers)

### Before publishing

Go through the hard rules in section zero first (dynamic library only; no
`rlib`/`.rmeta`/`.pdb`/`.d`; no packing by directory), then the rest:

- [ ] Does the query for `*.rlib`/`*.rmeta`/`*.pdb`/`*.d` over the bundle return
      **zero lines**? (the command is in 0.6)
- [ ] Does the bundle hold only the dynamic library (and the optional import
      library)?
- [ ] Does the deliverable crate's `crate-type` hold `cdylib` only? (a crate that
      also builds an `rlib` cannot be the deliverable)
- [ ] Was `cargo package` / `cargo publish` ever run against a crate that contains
      an implementation?
- [ ] Are `lto = "fat"` and `codegen-units = 1` on?
- [ ] Do panic and assertion messages leak module structure or business
      information?
- [ ] Are keys or licence checks inside the library? (putting them there publishes
      them)
- [ ] Was `panic = "abort"` turned on by mistake? (it breaks panic containment)
- [ ] Is packing or virtualisation used? (it conflicts with run-time loading;
      verify separately)
- [ ] If impersonation is the worry: is there a traceable build identifier? Is a
      contract in place and is the deliverable's hash registered?
- [ ] Is the premise accepted that "obfuscation raises the cost; it does not
      prevent reverse engineering"?

---

## 4. Shapes a declaration cannot have

`stable_object!` and `stable_object_guarded!` accept one slot shape; every other
shape is a compile error **at the declaration**, not at the first call. This
section collects each refusal, the message it produces, and the way to write it
instead.

### 4.1 A table lists the methods of one trait

```rust
pub trait Store: Send { /* ... */ }
// error: a table lists the methods of one trait, so a declaration cannot name a supertrait
```

An inherited method belongs to another declaration, and this table has no slot for
it. Two ways to fix it: repeat the inherited methods in this trait, or put the
object declaration on the trait that holds them so that the table lists
everything.

### 4.2 Three forms of receiver

`fn name(&self)`, `fn name(&mut self)` and `fn name(self)` are supported, followed
by comma-separated `argument: Type` pairs, an optional return type, and a `;` or a
default body. Every other shape reports
`a table slot has a fixed parameter list`:

| Form | Why not | What to write |
| --- | --- | --- |
| an associated constant or type | not a call | put it on an ordinary type |
| an associated function with no receiver | the table dispatches on the receiver | make it `&self`, or use a free function |
| a generic method `fn f<T>(...)` | one slot has one parameter list | write one concrete method per instantiation, forwarding to a generic method on the implementation |
| `-> Self` | `Self` is the implementation type, which the caller does not know | return a concrete boundary type |
| returning a borrow | a slot lends its value for that call only | return an owned value, or have the caller pass in storage it keeps |

### 4.3 A return type must be an owned value

A `'` in the return type reports `a method's return type names a lifetime`; a
borrowed type written in its shortest form (`StableStr`, `StableSlice<u8>`,
`StableSliceMut<u8>`, `StableCowStr`, `StableCowSlice<u8>`, `StableOsStr`,
`StablePath`, with a path prefix or without) reports
`a method cannot return a borrowed type`. The two checks differ only in spelling;
the reason is the same: once the call returns, the slot no longer owns that
storage. `'static` is refused under the same rule - data that is "static" in a
dynamic library is gone **after the unload**, and the type system is not told
about the unload, so even a form that could hold an owned value, such as
`StableCowStr<'static>`, cannot be written; return `StableString` when you mean its
owned form.

### 4.4 Argument and return types must be boundary types

Both sides have to implement `AbiType`, so writing `String`, `&str`, `usize` or
`bool` into a declaration produces an ordinary trait-bound error. Use
`StableString`, `StableStr`, `u64`, `StableBool` instead.

The signature also requires `AbiIdentity` on both sides: each slot records the
numbers of those types, not the way they were written. A boundary type of your own
has to supply it (`unsafe_stable_identity!(Type);` - the `unsafe` in the name is
the promise that both sides agree on what the name means), otherwise the
declaration fails where `AbiIdentity` is not satisfied; types generated by
`stable_enum!` already carry it.

### 4.5 A table holds at most 1024 methods

Above that a compile-time assertion fails:
`a declaration has more methods than a table can record`. A receiver reads at most
`MAX_SLOTS` (1024) entries, so any method listed beyond that is invisible to the
other side.

### 4.6 Attributes on methods

Attributes fall into two groups, and the macro places each where it can go:

| Attribute | Placed on | Effect |
| --- | --- | --- |
| `#[cfg(...)]` | the table field, the slot array element, the accessor, the trampoline | a method they remove produces no table entry, and `ABI_SLOT_SIGNATURES` loses one entry too (`tests/conditional.rs` pins this) |
| `#[cfg_attr(...)]` | only the generated functions (accessor, trampoline) | it may expand to something a field does not accept (`#[inline]`), so the macro does not put it on a field; to make a method disappear under a condition, write `#[cfg]` |
| every other attribute (`#[inline]`, `#[cold]`, `#[must_use]`...) and doc comments | only the generated functions (accessor, trampoline) | fields and array elements do not accept them, so the macro does not put them there |

The split separates "an attribute decides whether the method exists" from "an
attribute only affects a function": the former has to appear in every position
(otherwise a field and a table entry disagree), and the latter only produces
errors in the wrong position.
`a_method_may_carry_an_attribute_that_only_a_function_accepts` in
`tests/objects.rs` pins `#[inline]`.

Anything written in the attribute list of the `struct` / `trait` / `enum` itself
applies to **that declaration only**; the table, the impl and the accessors the
macro generates are not covered. So to gate a whole declaration, put the attribute
on the **macro invocation** (`#[cfg(...)] stable_object! { ... }`, which is legal),
not inside the declaration - otherwise the generated table refers to a type that
has been removed.

The receiver may be written `&self`, `&mut self`, `self`, and also `self: &Self`
or `self: &mut Self` (`the_receiver_and_the_argument_list_may_be_written_out` in
`tests/objects.rs` pins the last two). The argument list may end with a comma
(`fn f(&self, a: u32,)`), and all four receiver forms are accepted by both macros
(`stable_object!` and `stable_object_guarded!`). **The one form that is not
supported**: `&'a self` - its only use is returning a borrow, and a return type
that names a lifetime is exactly what this library refuses.

### 4.7 Limits of an enum declaration

`stable_enum!` writes a tag and a union, and therefore:

| Limit | Reason | What to write instead |
| --- | --- | --- |
| a payload cannot name a lifetime the declaration introduces | the generated type has no lifetime parameters, so only `'static` payloads can be written | use owned types (`StableString` rather than `StableStr<'a>`); `StableStr<'static>` works |
| a payload holds at most 12 fields | the stored form of two or more fields has to be a `#[repr(C)]` struct this library writes (`Payload2`...`Payload12`): a Rust tuple's layout is decided by the compiler and may change offsets between builds | collect the fields into a boundary struct |
| `stable_enum!` requires payloads to implement `Wipe` | the `Wipe` it writes wipes element by element, and the value still has to be releasable afterwards | when a payload cannot be wiped, use `stable_enum_unwiped!`: the same type without `Wipe` |
| discriminants must differ | two arms would read the same payload | a compile-time assertion refuses a repeat (`a declaration names one discriminant twice`) |
| comparison, copy, hash and the default have to be named | a `derive` reads the union rather than the payload the tag selects; and an `impl Ord` and the like require **every** payload to satisfy one set of bounds, so nothing is generated unless they are named | `derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash, StableHash];` (`Clone` also writes `TryClone`; `StableHash` lets the enumeration be a key for `insert_stable`); name the default with `default = Variant;`, and that variant cannot hold a payload |
| no named variant fields | the generated payload struct needs names, and `macro_rules!` cannot paste one identifier into another | use the tuple form; the fields of the view are positions (`payload.0`, `payload.1`) |
| `Eq` must be named together with `Ord` | equality being total is a property of the payload types, and the only property of them the macro can hand to the compiler for checking is the ordering | a declaration that names `Eq` without `Ord` is refused (there is a `compile_fail` case); if a payload is equal to itself without being ordered, write `impl Eq` yourself |
| `PanicFallback` has to be declared | there is no "no answer" variant to pick automatically | `stable_enum_fallback!(Name => Variant());` |

---

## 5. What this document does not cover

- None of the measurements above was repeated on another platform (Linux, macOS)
  or with another compiler version.
- Whether nightly's `-Zlocation-detail=none` removes the panic path was not
  verified.
- Throughput scaling across threads was not measured.
- No end-to-end numbers after batching are given - those depend on your kernel and
  your batch size; `demo` provides the instrument, not the conclusion.
