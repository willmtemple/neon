//! Safe _handles_ to managed JavaScript memory.

pub(crate) mod internal;

use self::internal::SuperType;
use context::Context;
use neon_runtime;
use neon_runtime::raw;
use result::{JsResult, JsResultExt};
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::os::raw::c_void;
use types::{JsValue, Value};

/// The trait of data that is managed by the JS garbage collector and can only be accessed via handles.
pub trait Managed: Copy {
    fn to_raw(self) -> raw::Local;

    fn from_raw(h: raw::Local) -> Self;
}

/// A safely rooted _handle_ to a JS value in memory that is managed by the garbage collector.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Handle<'a, T: Managed + 'a> {
    value: T,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: Managed + 'a> PartialEq for Handle<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        unsafe { neon_runtime::mem::same_handle(self.to_raw(), other.to_raw()) }
    }
}

impl<'a, T: Managed + 'a> Eq for Handle<'a, T> {}

impl<'a, T: Managed + 'a> Handle<'a, T> {
    pub(crate) fn new_internal(value: T) -> Handle<'a, T> {
        Handle {
            value: value,
            phantom: PhantomData,
        }
    }
}

/// An error representing a failed downcast.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct DowncastError<F: Value, T: Value> {
    phantom_from: PhantomData<F>,
    phantom_to: PhantomData<T>,
    description: String,
}

impl<F: Value, T: Value> Debug for DowncastError<F, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "DowncastError")
    }
}

impl<F: Value, T: Value> DowncastError<F, T> {
    fn new() -> Self {
        DowncastError {
            phantom_from: PhantomData,
            phantom_to: PhantomData,
            description: format!("failed downcast to {}", T::name()),
        }
    }
}

impl<F: Value, T: Value> Display for DowncastError<F, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.description())
    }
}

impl<F: Value, T: Value> Error for DowncastError<F, T> {
    fn description(&self) -> &str {
        &self.description
    }
}

/// The result of a call to `Handle::downcast()`.
pub type DowncastResult<'a, F, T> = Result<Handle<'a, T>, DowncastError<F, T>>;

impl<'a, F: Value, T: Value> JsResultExt<'a, T> for DowncastResult<'a, F, T> {
    fn or_throw<'b, C: Context<'b>>(self, cx: &mut C) -> JsResult<'a, T> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => cx.throw_type_error(&e.description),
        }
    }
}

impl<'a, T: Value> Handle<'a, T> {
    /// Safely upcast a handle to a supertype.
    ///
    /// This method does not require an execution context because it only copies a handle.
    pub fn upcast<U: Value + SuperType<T>>(&self) -> Handle<'a, U> {
        Handle::new_internal(SuperType::upcast_internal(self.value))
    }

    /// Tests whether this value is an instance of the given type.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// # use neon::prelude::*;
    /// # fn my_neon_function(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    /// let v: Handle<JsValue> = cx.number(17).upcast();
    /// v.is_a::<JsString>(); // false
    /// v.is_a::<JsNumber>(); // true
    /// v.is_a::<JsValue>();  // true
    /// # Ok(cx.undefined())
    /// # }
    /// ```
    pub fn is_a<U: Value>(&self) -> bool {
        U::is_typeof(self.value)
    }

    /// Attempts to downcast a handle to another type, which may fail. A failure
    /// to downcast **does not** throw a JavaScript exception, so it's OK to
    /// continue interacting with the JS engine if this method produces an `Err`
    /// result.
    pub fn downcast<U: Value>(&self) -> DowncastResult<'a, T, U> {
        match U::downcast(self.value) {
            Some(v) => Ok(Handle::new_internal(v)),
            None => Err(DowncastError::new()),
        }
    }

    /// Attempts to downcast a handle to another type, raising a JavaScript `TypeError`
    /// exception on failure. This method is a convenient shorthand, equivalent to
    /// `self.downcast::<U>().or_throw::<C>(cx)`.
    pub fn downcast_or_throw<'b, U: Value, C: Context<'b>>(&self, cx: &mut C) -> JsResult<'a, U> {
        self.downcast().or_throw(cx)
    }
}

impl<'a, T: Managed> Deref for Handle<'a, T> {
    type Target = T;
    fn deref<'b>(&'b self) -> &'b T {
        &self.value
    }
}

impl<'a, T: Managed> DerefMut for Handle<'a, T> {
    fn deref_mut<'b>(&'b mut self) -> &'b mut T {
        &mut self.value
    }
}

#[repr(C)]
pub struct PersistentHandle {
    ptr: *mut c_void,
}

unsafe impl Send for PersistentHandle {}

impl PersistentHandle {
    pub fn new<T: Value>(h: Handle<T>) -> PersistentHandle {
        let raw = h.value.to_raw();
        let ptr = unsafe { neon_runtime::mem::new_persistent(raw) };
        PersistentHandle { ptr: ptr }
    }

    pub fn into_handle<'a>(self) -> Handle<'a, JsValue> {
        unsafe {
            let mut local: raw::Local = mem::zeroed();
            neon_runtime::mem::new(&mut local, self.ptr);
            JsValue::new_internal(local)
        }
    }
}

impl Clone for PersistentHandle {
    fn clone(&self) -> Self {
        PersistentHandle {
            ptr: unsafe { neon_runtime::mem::clone_persistent(self.ptr) },
        }
    }
}

impl Drop for PersistentHandle {
    fn drop(&mut self) {
        unsafe { neon_runtime::mem::delete_persistent(self.ptr) }
    }
}
