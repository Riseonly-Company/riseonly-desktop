use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_void};

const RTLD_LAZY: c_int = 0x0001;
const FAMILY: &CStr = c"family";

#[repr(C)]
struct FcFontSet {
    nfont: c_int,
    _sfont: c_int,
    _fonts: *mut *mut c_void,
}

type NameParse = unsafe extern "C" fn(*const c_uchar) -> *mut c_void;
type ObjectSetCreate = unsafe extern "C" fn() -> *mut c_void;
type ObjectSetAdd = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FontList = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut FcFontSet;
type FontSetDestroy = unsafe extern "C" fn(*mut FcFontSet);
type Destroy = unsafe extern "C" fn(*mut c_void);

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct Fontconfig {
    name_parse: NameParse,
    object_set_create: ObjectSetCreate,
    object_set_add: ObjectSetAdd,
    font_list: FontList,
    set_destroy: FontSetDestroy,
    object_set_destroy: Destroy,
    pattern_destroy: Destroy,
}

pub(super) fn covered(samples: &[char]) -> Vec<bool> {
    let Some(fontconfig) = Fontconfig::open() else {
        return vec![false; samples.len()];
    };
    samples.iter().map(|sample| fontconfig.font_count(*sample).is_some_and(|count| count > 0)).collect()
}

impl Fontconfig {
    fn open() -> Option<Self> {
        let handle = unsafe { dlopen(c"libfontconfig.so.1".as_ptr(), RTLD_LAZY) };
        if handle.is_null() {
            return None;
        }
        let symbol = |name: &CStr| {
            let address = unsafe { dlsym(handle, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        };
        unsafe {
            Some(Self {
                name_parse: std::mem::transmute::<*mut c_void, NameParse>(symbol(c"FcNameParse")?),
                object_set_create: std::mem::transmute::<*mut c_void, ObjectSetCreate>(symbol(c"FcObjectSetCreate")?),
                object_set_add: std::mem::transmute::<*mut c_void, ObjectSetAdd>(symbol(c"FcObjectSetAdd")?),
                font_list: std::mem::transmute::<*mut c_void, FontList>(symbol(c"FcFontList")?),
                set_destroy: std::mem::transmute::<*mut c_void, FontSetDestroy>(symbol(c"FcFontSetDestroy")?),
                object_set_destroy: std::mem::transmute::<*mut c_void, Destroy>(symbol(c"FcObjectSetDestroy")?),
                pattern_destroy: std::mem::transmute::<*mut c_void, Destroy>(symbol(c"FcPatternDestroy")?),
            })
        }
    }

    fn font_count(&self, sample: char) -> Option<usize> {
        let query = CString::new(format!(":charset={:x}", u32::from(sample))).ok()?;
        unsafe {
            let pattern = (self.name_parse)(query.as_ptr().cast());
            if pattern.is_null() {
                return None;
            }
            let objects = (self.object_set_create)();
            if objects.is_null() {
                (self.pattern_destroy)(pattern);
                return None;
            }
            (self.object_set_add)(objects, FAMILY.as_ptr());
            let set = (self.font_list)(std::ptr::null_mut(), pattern, objects);
            (self.object_set_destroy)(objects);
            (self.pattern_destroy)(pattern);
            if set.is_null() {
                return None;
            }
            let count = usize::try_from((*set).nfont).unwrap_or(0);
            (self.set_destroy)(set);
            Some(count)
        }
    }
}
