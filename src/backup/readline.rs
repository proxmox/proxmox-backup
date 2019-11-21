//! Wrapper for GNU readline
//!
//! Implements a wrapper around the GNU readline C interface.
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::sync::Once;
use std::mem;

use libc;

use super::catalog::{CatalogReader, DirEntry};

#[link(name = "readline")]
extern "C" {
    fn readline(prompt: *const libc::c_char) -> *mut libc::c_char;
    fn add_history(line: *const libc::c_char);
    static mut rl_attempted_completion_function: extern "C" fn(
        text: *const libc::c_char,
        start: libc::c_int,
        end: libc::c_int,
    ) -> *const *const libc::c_char;
}

/// Context holding the catalog reader and stack of the current
/// working directory.
pub struct Context {
    pub catalog: CatalogReader<File>,
    pub current: Vec<DirEntry>,
}

pub struct Readline {
    prompt: CString,
    completion_callback: Option<Box<dyn Fn(&mut Context, &CStr, usize, usize) -> Vec<CString>>>,
    ctx: Option<Context>,
}

std::thread_local! {
    static CALLBACK: RefCell<Option<Box<dyn Fn(&mut Context, &CStr, usize, usize) -> Vec<CString>>>> = RefCell::new(None);
    static CONTEXT: RefCell<Option<Context>> = RefCell::new(None);
}

static mut INIT: Once = Once::new();

impl Readline {
    /// Create a new readline instance.
    ///
    /// This will create a readline instance, showing the given prompt and setting the completion
    /// callback to the provided function.
    pub fn new(
        prompt: CString,
        root: Vec<DirEntry>,
        completion_callback: Box<dyn Fn(&mut Context, &CStr, usize, usize) -> Vec<CString>>,
        catalog: CatalogReader<File>,
    ) -> Self {
        unsafe { INIT.call_once(||
            rl_attempted_completion_function = Self::attempted_completion
        )};
        Self {
            prompt,
            completion_callback: Some(completion_callback),
            ctx: Some(Context {
                catalog,
                current: root,
            })
        }
    }

    /// Wrapper function to provide the libc readline functionality.
    ///
    /// Prints the shell prompt and returns the line read from stdin.
    /// None is returned on EOF.
    pub fn readline(&mut self) -> Option<Vec<u8>>  {
        let pptr = self.prompt.as_ptr() as *const i8;
        let lptr = CALLBACK.with(|cb|
            CONTEXT.with(|ctx| {
                // Swap context and callback into thread local to be used in
                // readline rl_attempted_completion_function callback.
                mem::swap(&mut *cb.borrow_mut(), &mut self.completion_callback);
                mem::swap(&mut *ctx.borrow_mut(), &mut self.ctx);
                let lptr = unsafe { readline(pptr) };
                // Swap context and callback back into self.
                mem::swap(&mut *cb.borrow_mut(), &mut self.completion_callback);
                mem::swap(&mut *ctx.borrow_mut(), &mut self.ctx);
                lptr
            })
        );
        if lptr.is_null() {
            None
        } else {
            let slice = unsafe { CStr::from_ptr(lptr) };
            let line = slice.to_bytes().to_vec();
            unsafe {
                add_history(lptr as *const libc::c_char);
                libc::free(lptr as *mut libc::c_void);
            }
            Some(line)
        }
    }

    /// Sets the prompt to the provided string.
    pub fn update_prompt(&mut self, prompt: CString) {
        self.prompt = prompt;
    }

    /// Access the context from outside.
    pub fn context(&mut self) -> &mut Context {
        self.ctx.as_mut().unwrap()
    }

    /// Callback function for the readline C implementation.
    ///
    /// This will call the completion function registered on instance creation
    /// and pass it the context.
    /// It further converts the result returned by the callback to a FFI compatible
    /// list of pointers to CStrings.
    extern "C" fn attempted_completion(
          text: *const libc::c_char,
          start: libc::c_int,
          end: libc::c_int,
    ) -> *const *const libc::c_char {
        let list = CALLBACK.with(|cb| {
            CONTEXT.with(|ctx| {
                unsafe {
                    (*cb.borrow_mut().as_ref().unwrap())(
                        &mut (*ctx.borrow_mut().as_mut().unwrap()),
                        CStr::from_ptr(text),
                        start as usize,
                        end as usize
                    )
                }
            })
        });
        if list.is_empty() {
            return std::ptr::null();
        }
        // Create a list of pointers to the individual strings returnable via FFI
        let mut ptr_list: Vec<_>  = list.iter().map(|s| s.as_ptr()).collect();
        // Final pointer is null, end of list
        ptr_list.push(std::ptr::null());
        let ptr = ptr_list.as_ptr();
        // Pass ownership to caller
        std::mem::forget(list);
        std::mem::forget(ptr_list);
        ptr
    }
}
