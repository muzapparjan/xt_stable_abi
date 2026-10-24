# The demo: a kernel, a published surface, and three programs

**English** · [中文](README.zh-CN.md)

This directory holds the crates the ABI crate's own tests cannot stand in for,
because they are a real project rather than a fixture:

| crate | path | depends on | builds |
| --- | --- | --- | --- |
| `xt_kernel` | `kernel/` | nothing | an `rlib` |
| `xt_kernel_api` | `api/` | `xt_stable_abi` | an `rlib` |
| `xt_kernel_impl` | `impl/` | `xt_kernel`, `xt_kernel_api` | an `rlib` |
| `xt_kernel_dynlib` | `dynlib/` | `xt_kernel_impl`, `xt_kernel_api` | **only** a `cdylib` |
| `xt_kernel_app` | `app/` | `xt_kernel_api` | a binary |
| `xt_kernel_static` | `static/` | `xt_kernel_impl`, `xt_kernel_api` | a binary |
| `xt_kernel_native` | `native/` | `xt_kernel` | a binary |

Two more crates hold what the programs share, so that the comparison is between
paths and not between workloads:

| crate | path | holds |
| --- | --- | --- |
| `xt_bench` | `bench/` | the inputs, the expected answers, the timed rounds and the check report |
| `xt_surface_checks` | `surface-checks/` | the scenario list both boundary programs run |

The rule the table states is the point of the layout:

- the kernel does not depend on the ABI crate, and nothing in it mentions a
  stable layout, a descriptor or a dynamic library;
- the surface is written after the kernel and decides what leaves it: the
  kernel's borrowed tokens, its counted map, its ranking heap and its error enum
  stay inside, and a caller receives text, numbers, tuples, sequences, a time and
  an `AbiError` instead;
- the implementation depends on both and adds nothing of its own: every method
  forwards to the kernel and copies the answer into the published types;
- the dynamic library depends on the implementation and adds nothing of its own
  either: it exports two names and calls into the implementation;
- `xt_kernel_app` depends on the surface alone, so the kernel is not in its
  binary; `xt_kernel_static` adds the implementation so it can call the same
  entry point without a library file.

### Why the implementation is a separate crate

The implementation has to exist twice: once as an `rlib`, so a program can link
it and run the same protocol without a library file, and once as a `cdylib`, so a
host can load it at run time.

Putting both in one crate would make the deliverable also produce an `rlib`. That
is a leak, not a detail: an `rlib` carries `.rmeta`, which holds source-level
information including doc comments. So the two are split - `xt_kernel_impl`
builds the `rlib`, and `xt_kernel_dynlib` declares `crate-type = ["cdylib"]` and
contains two forwarding functions. An archive of the build directory therefore
cannot hand out the deliverable's own metadata.

The deliverable is the dynamic library and nothing else.
[`docs/boundary-guidance.md`](../docs/boundary-guidance.md) states the rule and
the measurements behind it.

## The three programs

The same scenarios, the same inputs, the same expected answers, three ways to
reach the kernel:

| program | what it links | what the call crosses |
| --- | --- | --- |
| `xt_kernel_native` | `xt_kernel` | nothing: an ordinary function call |
| `xt_kernel_static` | `xt_kernel_impl` as a library, `xt_kernel_api` | the published tables, linked in |
| `xt_kernel_app` | `xt_kernel_api` | the published tables, in a library opened at run time |

`xt_kernel_static` calls the very entry point `xt_kernel_app` looks up with
`GetProcAddress`/`dlsym`, so the protocol after the call is identical: descriptor,
adoption, then every accessor through the same tables. The only difference
between the two boundary programs is where those tables live.

Linking the implementation in does **not** turn the calls into direct calls. Both
boundary programs look up a slot in the object's table, call through the function
pointer stored there, and receive an owned boundary value allocated through the
host's allocator. What linking saves is the file and the two symbol lookups, once
at startup - so the two boundary programs match on every call, and only the
reference program is cheaper. [`docs/boundary-guidance.md`](../docs/boundary-guidance.md)
gives the numbers.

Putting the three outputs side by side therefore isolates two things at once:

- the cost of the boundary protocol itself, by comparing `xt_kernel_static` with
  `xt_kernel_native` (the `split_fixed` row separates the crossing - about 7 ns -
  from building a returned owned value - about 30 to 40 ns);
- the cost of arriving at the boundary through a file, by comparing
  `xt_kernel_app` with `xt_kernel_static` (measured: within noise).

The checks line up by name. The boundary programs add four checks the reference
cannot make - the descriptor's name, its version, the protocol version, and the
shutdown hook - and state the panic check differently, because in the reference
the panic is caught in the same process while in the boundary programs it is
contained on the library's side and reported as a status.

## Running them

```sh
cargo build --workspace
cargo run -p xt_kernel_native
cargo run -p xt_kernel_static
cargo run -p xt_kernel_app
```

`cargo run` puts the library next to the binary, which is where `xt_kernel_app`
looks first; a deployment does the same. A path may also be passed as the first
argument:

```sh
cargo run -p xt_kernel_app -- target/debug/xt_kernel_dynlib.dll
```

Each program prints one line per check, one line per timing, and ends with the
counts. It exits with a non-zero code when a check fails, so each can be a CI
step.

## What may be published

Only the dynamic library: `target/release/xt_kernel_dynlib.dll` (or `.so`,
`.dylib`).

```powershell
Get-ChildItem -Recurse <bundle> -Include *.rlib,*.rmeta,*.pdb,*.d
```

That query must return nothing. `target/release` after a full build returns 54
such files, including 12 `rlib`s; they belong to crates whose source is not meant
to leave the build machine. [`docs/boundary-guidance.md`](../docs/boundary-guidance.md)
explains why and states the rest of the rule.

## What it proves, and what it does not

The programs show that a value of every published shape survives the trip in both
directions: strings, sequences, tuples, a tagged result with an error, a
generated handle returned by another handle and passed back in, a panic contained
on the library's side, and the shutdown order when the plugin is dropped.

One answer cannot be reproduced across all three: a session that absorbs a peer
through a published handle learns the peer's name and total but not its texts, so
its closing line counts two texts where the reference counts three. The word
count is the same, which is what the published surface promises.

They do not test the kernel against hostile input, measure throughput against a
profiler, or exercise the surface on a second target. Those belong to the review
that follows, and the programs are the instrument for it.
