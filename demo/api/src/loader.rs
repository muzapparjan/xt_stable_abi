//! Opening a library and adopting the object it publishes.
//!
//! The loader is part of the published surface: a program that links this crate
//! can open the library, check that it speaks the same protocol version, and
//! receive a [`WorkspaceObject`] without knowing anything about the operating
//! system's loading API or about the kernel.

use core::ffi::c_void;
use core::mem;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::{
    AbiAllocator, AbiError, AbiVersion, EntryPointFn, HostServices, LoadedPlugin, WorkspaceObject,
    ABI_VERSION,
};

/// The way a library references one of its exported symbols.
type Symbol = *mut c_void;

/// A loaded dynamic library.
///
/// Dropping the value unloads the library. The value is held by the
/// [`LoadedPlugin`] a [`Plugin`] owns, so the library stays loaded until every
/// object it published has been released.
pub struct Library {
    module: Symbol,
    path: PathBuf,
}

impl Library {
    /// Opens the library at `path`.
    ///
    /// The path is made absolute before it reaches the loader. A name the
    /// loader resolves itself is searched for beside the working directory and
    /// on the platform's search path, so a file planted in either place is
    /// loaded instead of the library the host named; an absolute path removes
    /// that search. A path that carries an interior NUL is refused, because a
    /// C string ends there and the loader would open a different file.
    ///
    /// # Errors
    ///
    /// The operating system's error when the file cannot be loaded.
    pub fn open(path: &Path) -> io::Result<Self> {
        let absolute = std::path::absolute(path)?;
        let module = platform::open(&absolute)?;
        Ok(Self {
            module,
            path: absolute,
        })
    }

    /// The address of `symbol`, or null when the library does not export it.
    pub fn symbol(&self, symbol: &str) -> Symbol {
        platform::lookup(self.module, symbol)
    }

    /// The path the library was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for Library {
    // The module handle is an address in this process; the path is what a reader
    // needs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Library")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        platform::close(self.module);
    }
}

/// Why a library could not be used.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// The file could not be opened.
    Open {
        /// The path that was tried.
        path: PathBuf,
        /// The operating system's error.
        source: io::Error,
    },
    /// The library does not export the entry point.
    MissingEntry {
        /// The path that was loaded.
        path: PathBuf,
        /// The symbol that was looked up.
        symbol: &'static str,
    },
    /// The library answered with a failure, or published a table this build
    /// cannot use.
    Abi(AbiError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, source } => {
                write!(formatter, "could not open {}: {source}", path.display())
            }
            Self::MissingEntry { path, symbol } => write!(
                formatter,
                "{} does not export {symbol}",
                path.display()
            ),
            Self::Abi(error) => write!(formatter, "the library was refused: {error}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } => Some(source),
            Self::MissingEntry { .. } | Self::Abi(_) => None,
        }
    }
}

/// A loaded library together with the object it published.
///
/// The field order is the drop order a host needs: the object is released while
/// the library is still loaded, and only then is the library unloaded.
///
/// Dropping the value unloads the library, so the host owes it what it owes any
/// module it unloads: every object the library handed out is released first, the
/// library's own threads have stopped (its shutdown hook is where it joins
/// them), callbacks registered with the host are removed, and no call is in
/// progress. This type enforces the first of those for the object it holds and
/// nothing else.
pub struct Plugin {
    workspace: WorkspaceObject,
    loaded: LoadedPlugin,
}

impl Plugin {
    /// Opens `path`, calls its entry point and adopts the object it publishes.
    ///
    /// The host offers the library the process allocator and requires the
    /// library to implement at least this build's [`ABI_VERSION`]. A library
    /// built against an older minor of the same major is refused here; a
    /// library built against a newer minor is accepted, because the major
    /// number and the layout revision are what decide whether the fields are
    /// read correctly.
    ///
    /// # Errors
    ///
    /// [`LoadError::Open`] when the file cannot be loaded,
    /// [`LoadError::MissingEntry`] when the symbol is absent, and
    /// [`LoadError::Abi`] when the library reports a failure or publishes a
    /// table built from another declaration.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        Self::load_with_required(path, ABI_VERSION)
    }

    /// Opens `path` and requires the library to implement at least `required`.
    ///
    /// `required` has to name this build's major number: a library of another
    /// major version may give an existing field another meaning, and no value
    /// it publishes can be read under that doubt.
    ///
    /// # Errors
    ///
    /// The same as [`Plugin::load`], plus the version mismatch it reports when
    /// the library implements an older minor than `required`.
    pub fn load_with_required(path: &Path, required: AbiVersion) -> Result<Self, LoadError> {
        let library = Library::open(path).map_err(|source| LoadError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let symbol = library.symbol(crate::ENTRY_SYMBOL);
        if symbol.is_null() {
            return Err(LoadError::MissingEntry {
                path: path.to_path_buf(),
                symbol: crate::ENTRY_SYMBOL,
            });
        }
        // SAFETY: the symbol is the entry point the protocol names; a library
        // that exports something else under this name breaks the protocol, which
        // is exactly what the version check below detects before it is called.
        let entry: EntryPointFn = unsafe { mem::transmute::<Symbol, EntryPointFn>(symbol) };
        let host = HostServices::new(AbiAllocator::system());
        // SAFETY: the library was built against this crate's protocol, and the
        // host description outlives the call.
        let descriptor = unsafe { xt_stable_abi::load(entry, &host, required) }
            .map_err(LoadError::Abi)?;
        let mut loaded = LoadedPlugin::with_keep_alive(descriptor, library);
        let workspace =
            WorkspaceObject::abi_try_from_stable(loaded.take_api()).map_err(LoadError::Abi)?;
        Ok(Self { workspace, loaded })
    }

    /// The object the library published.
    pub fn workspace(&self) -> &WorkspaceObject {
        &self.workspace
    }

    /// The name the library stored in its descriptor.
    pub fn name(&self) -> &str {
        self.loaded.name()
    }

    /// The library's own version.
    pub fn version(&self) -> AbiVersion {
        self.loaded.descriptor().plugin_version
    }

    /// The version of the protocol the library was built against.
    pub fn protocol(&self) -> AbiVersion {
        self.loaded.descriptor().header.version
    }
}

impl fmt::Debug for Plugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Plugin")
            .field("name", &self.name())
            .field("version", &self.version())
            .finish_non_exhaustive()
    }
}

#[cfg(windows)]
mod platform {
    use super::{io, Path, Symbol};
    use core::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    /// Look for the library's own imports beside it rather than in the
    /// directories the process happens to search.
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x0000_0100;
    /// Look for the imports the system provides in `System32`.
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

    pub fn open(path: &Path) -> io::Result<Symbol> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        // A C string ends at the first zero, so a path that carries one names a
        // different file than the caller asked for.
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the library path contains a NUL",
            ));
        }
        wide.push(0);
        // SAFETY: the path is a NUL-terminated wide string that outlives the
        // call, and the flags name the two places a dependency is looked for.
        let module = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                core::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(module)
    }

    pub fn lookup(module: Symbol, name: &str) -> Symbol {
        let mut text: Vec<u8> = name.as_bytes().to_vec();
        text.push(0);
        // SAFETY: the module is live and the name is NUL-terminated.
        unsafe { GetProcAddress(module, text.as_ptr()) }
    }

    pub fn close(module: Symbol) {
        // SAFETY: the module came from `open` and is closed once.
        unsafe { FreeLibrary(module) };
    }
}

#[cfg(unix)]
mod platform {
    use super::{io, Path, Symbol};
    use core::ffi::{c_char, c_int};
    use std::ffi::CString;

    const RTLD_NOW: c_int = 2;

    unsafe extern "C" {
        fn dlopen(name: *const c_char, mode: c_int) -> *mut core::ffi::c_void;
        fn dlsym(handle: *mut core::ffi::c_void, name: *const c_char) -> *mut core::ffi::c_void;
        fn dlclose(handle: *mut core::ffi::c_void) -> c_int;
    }

    pub fn open(path: &Path) -> io::Result<Symbol> {
        // A path is bytes on this platform, and replacing the ones that are not
        // UTF-8 would open a different file or fail on a path that exists.
        use std::os::unix::ffi::OsStrExt;
        let text = CString::new(path.as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: the path is a NUL-terminated string that outlives the call.
        let module = unsafe { dlopen(text.as_ptr(), RTLD_NOW) };
        if module.is_null() {
            return Err(io::Error::other(format!(
                "dlopen refused {}",
                path.display()
            )));
        }
        Ok(module)
    }

    pub fn lookup(module: Symbol, name: &str) -> Symbol {
        let text = match CString::new(name) {
            Ok(text) => text,
            Err(_) => return core::ptr::null_mut(),
        };
        // SAFETY: the module is live and the name is NUL-terminated.
        unsafe { dlsym(module, text.as_ptr()) }
    }

    pub fn close(module: Symbol) {
        // SAFETY: the module came from `open` and is closed once.
        unsafe { dlclose(module) };
    }
}
