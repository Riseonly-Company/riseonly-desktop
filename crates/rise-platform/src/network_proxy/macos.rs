use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Arc;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{ns_string, NSArray, NSDictionary, NSError, NSNumber, NSString, NSURL};

use super::network_proxy::*;
use crate::system_activity::ActivityWatch;

type CfAllocatorRef = *const c_void;
type DispatchQueue = *mut c_void;
type ScDynamicStoreRef = *mut c_void;
type CfArrayRef = *const c_void;
type CfDictionaryRef = *const c_void;
type CfErrorRef = *const c_void;
type CfRunLoopRef = *mut c_void;
type CfRunLoopSourceRef = *mut c_void;
type CfStringRef = *const c_void;
type CfTypeRef = *const c_void;
type CfUrlRef = *const c_void;

type AutoConfigCallback =
    unsafe extern "C" fn(client: *mut c_void, proxy_list: CfArrayRef, error: CfErrorRef);

/// stream client context, version 0 and info goes to the callback
/// the other callbacks are null since info outlives the source
#[repr(C)]
struct CfStreamClientContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(info: *mut c_void) -> *mut c_void>,
    release: Option<unsafe extern "C" fn(info: *mut c_void)>,
    copy_description: Option<unsafe extern "C" fn(info: *mut c_void) -> CfStringRef>,
}

type DynamicStoreCallback =
    unsafe extern "C" fn(store: ScDynamicStoreRef, changed_keys: CfArrayRef, info: *mut c_void);

#[repr(C)]
struct ScDynamicStoreContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(info: *const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(info: *const c_void)>,
    copy_description: Option<unsafe extern "C" fn(info: *const c_void) -> CfStringRef>,
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    #[link_name = "SCDynamicStoreCreate"]
    fn sc_dynamic_store_create(
        allocator: CfAllocatorRef,
        name: CfStringRef,
        callout: DynamicStoreCallback,
        context: *const ScDynamicStoreContext,
    ) -> ScDynamicStoreRef;
    #[link_name = "SCDynamicStoreKeyCreateProxies"]
    fn sc_dynamic_store_key_create_proxies(allocator: CfAllocatorRef) -> CfStringRef;
    #[link_name = "SCDynamicStoreSetNotificationKeys"]
    fn sc_dynamic_store_set_notification_keys(
        store: ScDynamicStoreRef,
        keys: CfArrayRef,
        patterns: CfArrayRef,
    ) -> u8;
    #[link_name = "SCDynamicStoreSetDispatchQueue"]
    fn sc_dynamic_store_set_dispatch_queue(store: ScDynamicStoreRef, queue: DispatchQueue) -> u8;
    #[link_name = "SCError"]
    fn sc_error() -> c_int;
    #[link_name = "SCErrorString"]
    fn sc_error_string(status: c_int) -> *const c_char;
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> DispatchQueue;
    fn dispatch_async_f(
        queue: DispatchQueue,
        context: *mut c_void,
        work: unsafe extern "C" fn(context: *mut c_void),
    );
    fn dispatch_release(object: DispatchQueue);
}

#[link(name = "CFNetwork", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CFNetworkCopySystemProxySettings"]
    fn cf_network_copy_system_proxy_settings() -> CfDictionaryRef;
    #[link_name = "CFNetworkCopyProxiesForURL"]
    fn cf_network_copy_proxies_for_url(url: CfUrlRef, proxy_settings: CfDictionaryRef)
    -> CfArrayRef;
    #[link_name = "CFNetworkExecuteProxyAutoConfigurationURL"]
    fn cf_network_execute_proxy_auto_configuration_url(
        proxy_auto_config_url: CfUrlRef,
        target_url: CfUrlRef,
        callback: AutoConfigCallback,
        client_context: *mut CfStreamClientContext,
    ) -> CfRunLoopSourceRef;
    #[link_name = "CFNetworkExecuteProxyAutoConfigurationScript"]
    fn cf_network_execute_proxy_auto_configuration_script(
        proxy_auto_configuration_script: CfStringRef,
        target_url: CfUrlRef,
        callback: AutoConfigCallback,
        client_context: *mut CfStreamClientContext,
    ) -> CfRunLoopSourceRef;

    #[link_name = "kCFProxyTypeKey"]
    static CF_PROXY_TYPE_KEY: &'static NSString;
    #[link_name = "kCFProxyHostNameKey"]
    static CF_PROXY_HOST_NAME_KEY: &'static NSString;
    #[link_name = "kCFProxyPortNumberKey"]
    static CF_PROXY_PORT_NUMBER_KEY: &'static NSString;
    #[link_name = "kCFProxyUsernameKey"]
    static CF_PROXY_USERNAME_KEY: &'static NSString;
    #[link_name = "kCFProxyPasswordKey"]
    static CF_PROXY_PASSWORD_KEY: &'static NSString;
    #[link_name = "kCFProxyAutoConfigurationURLKey"]
    static CF_PROXY_AUTO_CONFIGURATION_URL_KEY: &'static NSString;
    #[link_name = "kCFProxyAutoConfigurationJavaScriptKey"]
    static CF_PROXY_AUTO_CONFIGURATION_JAVA_SCRIPT_KEY: &'static NSString;
    #[link_name = "kCFProxyTypeNone"]
    static CF_PROXY_TYPE_NONE: &'static NSString;
    #[link_name = "kCFProxyTypeHTTP"]
    static CF_PROXY_TYPE_HTTP: &'static NSString;
    #[link_name = "kCFProxyTypeHTTPS"]
    static CF_PROXY_TYPE_HTTPS: &'static NSString;
    #[link_name = "kCFProxyTypeSOCKS"]
    static CF_PROXY_TYPE_SOCKS: &'static NSString;
    #[link_name = "kCFProxyTypeAutoConfigurationURL"]
    static CF_PROXY_TYPE_AUTO_CONFIGURATION_URL: &'static NSString;
    #[link_name = "kCFProxyTypeAutoConfigurationJavaScript"]
    static CF_PROXY_TYPE_AUTO_CONFIGURATION_JAVA_SCRIPT: &'static NSString;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CFRunLoopGetCurrent"]
    fn cf_run_loop_get_current() -> CfRunLoopRef;
    #[link_name = "CFRunLoopAddSource"]
    fn cf_run_loop_add_source(run_loop: CfRunLoopRef, source: CfRunLoopSourceRef, mode: CfStringRef);
    #[link_name = "CFRunLoopRunInMode"]
    fn cf_run_loop_run_in_mode(mode: CfStringRef, seconds: f64, return_after_source: u8) -> i32;
    #[link_name = "CFRunLoopStop"]
    fn cf_run_loop_stop(run_loop: CfRunLoopRef);
    #[link_name = "CFRunLoopSourceInvalidate"]
    fn cf_run_loop_source_invalidate(source: CfRunLoopSourceRef);
    #[link_name = "CFRelease"]
    fn cf_release(object: CfTypeRef);
    #[link_name = "CFEqual"]
    fn cf_equal(first: CfTypeRef, second: CfTypeRef) -> u8;
}

/// the dictionary every lookup reads, proxies of the current network service
fn system_settings() -> Result<Retained<AnyObject>, ProxyUnavailable> {
    unsafe { Retained::from_raw(cf_network_copy_system_proxy_settings().cast_mut().cast::<AnyObject>()) }
        .ok_or_else(|| refused("CFNetworkCopySystemProxySettings returned NULL"))
}

/// state of one watch, shared by the dynamic store and the private serial queue
struct SettingsFollower {
    sink: ProxySettingsSink,
    known: parking_lot::Mutex<Option<Retained<AnyObject>>>,
}

unsafe impl Send for SettingsFollower {}
unsafe impl Sync for SettingsFollower {}

impl SettingsFollower {
    fn read_baseline(&self) {
        let mut known = self.known.lock();
        if known.is_some() {
            return;
        }
        match system_settings() {
            Ok(current) => *known = Some(current),
            Err(failure) => tracing::warn!(target: "riseonly", %failure, "no baseline of the proxy settings"),
        }
    }

    fn settings_changed(&self) {
        let current = match system_settings() {
            Ok(current) => current,
            Err(failure) => {
                tracing::warn!(target: "riseonly", %failure, "the changed proxy settings are not readable");
                return;
            }
        };
        let mut known = self.known.lock();
        let same = known.as_ref().is_some_and(|known| {
            unsafe { cf_equal(Retained::as_ptr(known).cast(), Retained::as_ptr(&current).cast()) != 0 }
        });
        if same {
            return;
        }
        *known = Some(current);
        drop(known);
        (self.sink)();
    }
}

unsafe extern "C" fn retain_follower(info: *const c_void) -> *const c_void {
    unsafe { Arc::increment_strong_count(info.cast::<SettingsFollower>()) };
    info
}

unsafe extern "C" fn release_follower(info: *const c_void) {
    unsafe { Arc::decrement_strong_count(info.cast::<SettingsFollower>()) };
}

unsafe extern "C" fn proxies_key_changed(
    _store: ScDynamicStoreRef,
    _changed_keys: CfArrayRef,
    info: *mut c_void,
) {
    let follower = unsafe { &*info.cast_const().cast::<SettingsFollower>() };
    follower.settings_changed();
}

unsafe extern "C" fn read_baseline_on_queue(context: *mut c_void) {
    let follower = unsafe { Arc::from_raw(context.cast_const().cast::<SettingsFollower>()) };
    follower.read_baseline();
}

struct SettingsWatch {
    store: ScDynamicStoreRef,
    queue: DispatchQueue,
}

unsafe impl Send for SettingsWatch {}

impl SettingsWatch {
    fn start(follower: &Arc<SettingsFollower>) -> Result<Self, ProxyUnavailable> {
        let queue =
            unsafe { dispatch_queue_create(c"com.riseonly.proxy-settings".as_ptr(), std::ptr::null()) };
        if queue.is_null() {
            return Err(refused("dispatch_queue_create returned NULL"));
        }
        let context = ScDynamicStoreContext {
            version: 0,
            info: Arc::as_ptr(follower).cast_mut().cast(),
            retain: Some(retain_follower),
            release: Some(release_follower),
            copy_description: None,
        };
        let name: &NSString = ns_string!("com.riseonly.proxy-settings");
        let store = unsafe {
            sc_dynamic_store_create(
                std::ptr::null(),
                std::ptr::from_ref(name).cast(),
                proxies_key_changed,
                &context,
            )
        };
        if store.is_null() {
            unsafe { dispatch_release(queue) };
            return Err(store_refused("SCDynamicStoreCreate"));
        }
        let watch = Self { store, queue };
        let key = unsafe {
            Retained::from_raw(sc_dynamic_store_key_create_proxies(std::ptr::null()).cast_mut().cast::<NSString>())
        }
        .ok_or_else(|| store_refused("SCDynamicStoreKeyCreateProxies"))?;
        let keys = NSArray::from_slice(&[&*key]);
        let keys_set = unsafe {
            sc_dynamic_store_set_notification_keys(store, Retained::as_ptr(&keys).cast(), std::ptr::null())
        };
        if keys_set == 0 {
            return Err(store_refused("SCDynamicStoreSetNotificationKeys"));
        }
        if unsafe { sc_dynamic_store_set_dispatch_queue(store, queue) } == 0 {
            return Err(store_refused("SCDynamicStoreSetDispatchQueue"));
        }
        unsafe {
            dispatch_async_f(
                queue,
                Arc::into_raw(Arc::clone(follower)).cast_mut().cast(),
                read_baseline_on_queue,
            );
        }
        Ok(watch)
    }
}

impl Drop for SettingsWatch {
    fn drop(&mut self) {
        unsafe {
            sc_dynamic_store_set_dispatch_queue(self.store, std::ptr::null_mut());
            cf_release(self.store.cast_const());
            dispatch_release(self.queue);
        }
    }
}

/// what failed, SCError holds the reason of the last system configuration call on this thread
fn store_refused(what: &str) -> ProxyUnavailable {
    let detail = unsafe { CStr::from_ptr(sc_error_string(sc_error())) };
    refused(&format!("{what}: {}", detail.to_string_lossy()))
}

const DEFAULT_HTTP_PROXY_PORT: u16 = 80;
const DEFAULT_SOCKS_PROXY_PORT: u16 = 1080;

pub(super) struct NativeProxySource;

impl SystemProxySource for NativeProxySource {
    fn watch_settings(&self, sink: ProxySettingsSink) -> Result<ActivityWatch, ProxyUnavailable> {
        let follower = Arc::new(SettingsFollower {
            sink,
            known: parking_lot::Mutex::new(None),
        });
        Ok(ActivityWatch::new(SettingsWatch::start(&follower)?))
    }

    fn proxies_for(&self, target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        let url = lookup_url(target)?;
        let settings = system_settings()?;
        let list = unsafe {
            Retained::from_raw(
                cf_network_copy_proxies_for_url(
                    Retained::as_ptr(&url).cast(),
                    Retained::as_ptr(&settings).cast(),
                )
                .cast_mut()
                .cast::<NSArray<AnyObject>>(),
            )
        }
        .ok_or_else(|| refused("CFNetworkCopyProxiesForURL returned NULL"))?;
        Ok(entries(&list))
    }

    fn run_auto_config(
        &self,
        config: &AutoConfig,
        target: &ProxyTarget,
        timeout: Duration,
    ) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        let target_url = lookup_url(target)?;
        let mut slot = AutoConfigSlot { outcome: None };
        let mut context = CfStreamClientContext {
            version: 0,
            info: std::ptr::from_mut(&mut slot).cast(),
            retain: None,
            release: None,
            copy_description: None,
        };
        let source = match config {
            AutoConfig::Url(address) => {
                let pac_url = NSURL::URLWithString(&NSString::from_str(address)).ok_or_else(|| {
                    ProxyUnavailable::AutoConfigFailed(format!("not a URL: {address}"))
                })?;
                unsafe {
                    cf_network_execute_proxy_auto_configuration_url(
                        Retained::as_ptr(&pac_url).cast(),
                        Retained::as_ptr(&target_url).cast(),
                        auto_config_finished,
                        &mut context,
                    )
                }
            }
            AutoConfig::Script(script) => {
                let script = NSString::from_str(script);
                unsafe {
                    cf_network_execute_proxy_auto_configuration_script(
                        Retained::as_ptr(&script).cast(),
                        Retained::as_ptr(&target_url).cast(),
                        auto_config_finished,
                        &mut context,
                    )
                }
            }
        };
        if source.is_null() {
            return Err(refused("the PAC run loop source could not be created"));
        }
        let mode: &NSString = ns_string!("com.riseonly.proxy-auto-config");
        let mode_ref: CfStringRef = std::ptr::from_ref(mode).cast();
        unsafe {
            cf_run_loop_add_source(cf_run_loop_get_current(), source, mode_ref);
            cf_run_loop_run_in_mode(mode_ref, timeout.as_secs_f64(), 1);
            cf_run_loop_source_invalidate(source);
            cf_release(source.cast_const());
        }
        slot.outcome.unwrap_or(Err(ProxyUnavailable::AutoConfigTimedOut(timeout)))
    }
}

/// what the pac callback leaves for the waiting lookup
struct AutoConfigSlot {
    outcome: Option<Result<Vec<SystemProxyEntry>, ProxyUnavailable>>,
}

unsafe extern "C" fn auto_config_finished(
    client: *mut c_void,
    proxy_list: CfArrayRef,
    error: CfErrorRef,
) {
    let slot = unsafe { &mut *client.cast::<AutoConfigSlot>() };
    slot.outcome = Some(if !error.is_null() {
        let error = unsafe { &*error.cast::<NSError>() };
        Err(ProxyUnavailable::AutoConfigFailed(error.localizedDescription().to_string()))
    } else if proxy_list.is_null() {
        Err(ProxyUnavailable::AutoConfigFailed(String::from("the PAC run returned no list")))
    } else {
        let list = unsafe { &*proxy_list.cast::<NSArray<AnyObject>>() };
        Ok(entries(list))
    });
    unsafe { cf_run_loop_stop(cf_run_loop_get_current()) };
}

fn lookup_url(target: &ProxyTarget) -> Result<Retained<NSURL>, ProxyUnavailable> {
    let address = target.lookup_url();
    NSURL::URLWithString(&NSString::from_str(&address))
        .ok_or_else(|| refused(&format!("not a URL: {address}")))
}

fn refused(detail: &str) -> ProxyUnavailable {
    ProxyUnavailable::SystemRefused(detail.to_owned())
}

/// ordered proxy dictionaries as seam entries, ftp and unknown types are skipped
fn entries(list: &NSArray<AnyObject>) -> Vec<SystemProxyEntry> {
    list.iter()
        .filter_map(|item| {
            let proxy = item.downcast_ref::<NSDictionary>()?;
            let proxy = unsafe { &*std::ptr::from_ref(proxy).cast::<NSDictionary<NSString, AnyObject>>() };
            entry(proxy)
        })
        .collect()
}

fn entry(proxy: &NSDictionary<NSString, AnyObject>) -> Option<SystemProxyEntry> {
    let (type_key, none, http, https, socks, pac_url, pac_script) = unsafe {
        (
            CF_PROXY_TYPE_KEY,
            CF_PROXY_TYPE_NONE,
            CF_PROXY_TYPE_HTTP,
            CF_PROXY_TYPE_HTTPS,
            CF_PROXY_TYPE_SOCKS,
            CF_PROXY_TYPE_AUTO_CONFIGURATION_URL,
            CF_PROXY_TYPE_AUTO_CONFIGURATION_JAVA_SCRIPT,
        )
    };
    let kind = proxy.objectForKey(type_key)?;
    let kind = kind.downcast_ref::<NSString>()?;
    if kind.isEqualToString(none) {
        return Some(SystemProxyEntry::Direct);
    }
    if kind.isEqualToString(http) || kind.isEqualToString(https) {
        return endpoint(proxy, ProxyProtocol::Http, DEFAULT_HTTP_PROXY_PORT).map(SystemProxyEntry::Proxy);
    }
    if kind.isEqualToString(socks) {
        return endpoint(proxy, ProxyProtocol::Socks5, DEFAULT_SOCKS_PROXY_PORT)
            .map(SystemProxyEntry::Proxy);
    }
    if kind.isEqualToString(pac_url) {
        let key = unsafe { CF_PROXY_AUTO_CONFIGURATION_URL_KEY };
        let value = proxy.objectForKey(key)?;
        let address = value.downcast_ref::<NSURL>()?.absoluteString()?.to_string();
        return Some(SystemProxyEntry::AutoConfig(AutoConfig::Url(address)));
    }
    if kind.isEqualToString(pac_script) {
        let key = unsafe { CF_PROXY_AUTO_CONFIGURATION_JAVA_SCRIPT_KEY };
        let script = text(proxy, key)?;
        return Some(SystemProxyEntry::AutoConfig(AutoConfig::Script(script)));
    }
    None
}

fn endpoint(
    proxy: &NSDictionary<NSString, AnyObject>,
    protocol: ProxyProtocol,
    default_port: u16,
) -> Option<ProxyEndpoint> {
    let (host_key, port_key, user_key, password_key) = unsafe {
        (CF_PROXY_HOST_NAME_KEY, CF_PROXY_PORT_NUMBER_KEY, CF_PROXY_USERNAME_KEY, CF_PROXY_PASSWORD_KEY)
    };
    let host = text(proxy, host_key).filter(|host| !host.is_empty())?;
    let port = proxy
        .objectForKey(port_key)
        .and_then(|value| value.downcast_ref::<NSNumber>().map(NSNumber::integerValue))
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .unwrap_or(default_port);
    let mut endpoint = ProxyEndpoint::new(protocol, &host, port);
    endpoint.credentials = text(proxy, user_key).filter(|user| !user.is_empty()).map(|user| {
        ProxyCredentials {
            user,
            password: text(proxy, password_key).unwrap_or_default(),
        }
    });
    Some(endpoint)
}

fn text(proxy: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<String> {
    let value = proxy.objectForKey(key)?;
    Some(value.downcast_ref::<NSString>()?.to_string())
}
