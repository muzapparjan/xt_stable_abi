//! The shutdown step a host runs before unloading a library.

use std::sync::atomic::{AtomicUsize, Ordering};

use xt_stable_abi::{
    AbiVersion, LoadedPlugin, PluginDescriptor, StableDyn,
};

static SHUTDOWNS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counting_shutdown() {
    SHUTDOWNS.fetch_add(1, Ordering::SeqCst);
}

fn descriptor_with_shutdown() -> PluginDescriptor {
    let mut descriptor = PluginDescriptor::buffer();
    // SAFETY: the buffer is valid and large enough.
    let status = unsafe {
        PluginDescriptor::store(
            &mut descriptor,
            "",
            AbiVersion::zero(),
            StableDyn::empty(),
            Some(counting_shutdown),
        )
    };
    assert!(status.is_ok());
    descriptor
}

#[test]
fn a_buffer_starts_without_a_shutdown_hook() {
    let mut descriptor = PluginDescriptor::buffer();
    // Running the default hook does nothing and is allowed.
    descriptor.shutdown();
}

#[test]
fn the_recorded_hook_runs_once() {
    let before = SHUTDOWNS.load(Ordering::SeqCst);
    let mut descriptor = descriptor_with_shutdown();
    descriptor.shutdown();
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before + 1);
    // A second call replaces the hook with the default one.
    descriptor.shutdown();
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before + 1);
}

#[test]
fn writing_a_descriptor_records_the_hook() {
    let mut descriptor = PluginDescriptor::buffer();
    // SAFETY: the buffer is valid and large enough.
    let status = unsafe {
        PluginDescriptor::store(
            &mut descriptor,
            "named",
            AbiVersion::new(1, 0),
            StableDyn::empty(),
            Some(counting_shutdown),
        )
    };
    assert!(status.is_ok());
    assert_eq!(descriptor.name_str(), "named");
    let before = SHUTDOWNS.load(Ordering::SeqCst);
    descriptor.shutdown();
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before + 1);
}

#[test]
fn a_loaded_plugin_runs_the_hook_when_it_goes_away() {
    let before = SHUTDOWNS.load(Ordering::SeqCst);
    {
        let plugin = LoadedPlugin::new(descriptor_with_shutdown());
        assert_eq!(plugin.name(), "");
    }
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before + 1);
}

#[test]
fn taking_the_descriptor_out_of_a_loaded_plugin_runs_the_hook() {
    let before = SHUTDOWNS.load(Ordering::SeqCst);
    let plugin = LoadedPlugin::new(descriptor_with_shutdown());
    let descriptor = plugin.into_descriptor();
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before + 1);
    drop(descriptor);
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before + 1);
}

#[test]
fn wiping_a_descriptor_wipes_what_it_holds() {
    use xt_stable_abi::Wipe;

    let mut descriptor = PluginDescriptor::buffer();
    // SAFETY: the buffer is valid and large enough.
    let status = unsafe {
        PluginDescriptor::store(
            &mut descriptor,
            "named",
            AbiVersion::new(1, 2),
            StableDyn::empty(),
            Some(counting_shutdown),
        )
    };
    assert!(status.is_ok());
    assert_eq!(descriptor.name_str(), "named");

    descriptor.wipe_in_place();

    assert!(descriptor.shutdown.is_none());
    assert_eq!(descriptor.header.capacity, 0);
    assert_eq!(descriptor.header.written, 0);
    assert_eq!(descriptor.plugin_version, AbiVersion::zero());
    // The name keeps its shape and loses its content: a wipe overwrites the
    // bytes, it does not make the value unreadable.
    assert_eq!(descriptor.name_str().len(), "named".len());
    assert!(
        descriptor.name_str().bytes().all(|byte| byte == 0),
        "every byte of the name is overwritten"
    );
    assert!(descriptor.take_api().is_empty());

    // The hook belongs to the library, not to the content of this value, so
    // wiping the descriptor does not run it.
    let before = SHUTDOWNS.load(Ordering::SeqCst);
    descriptor.shutdown();
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), before);
}
