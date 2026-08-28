//! Minimal OpenCL binding.
//!
//! The OpenCL ICD loader (`OpenCL.dll` / `libOpenCL.so.1`) is *dlopen*ed at
//! runtime, not linked — so the binary starts fine on a machine with no OpenCL
//! at all, and the GPU benchmark simply reports itself unavailable. We bind only
//! the ~20 entry points the benchmark needs, with thin `Result`-returning
//! wrappers and RAII handles.

#![allow(non_camel_case_types)]

use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use libloading::{Library, Symbol};

// --- OpenCL C ABI types --------------------------------------------------------

pub type cl_int = i32;
pub type cl_uint = u32;
pub type cl_ulong = u64;
pub type cl_bitfield = u64;
pub type cl_device_type = cl_bitfield;
pub type cl_mem_flags = cl_bitfield;
pub type cl_command_queue_properties = cl_bitfield;

type cl_platform_id = *mut c_void;
type cl_device_id = *mut c_void;
/// An opaque device handle, passed straight back to the driver. `Copy`; never
/// dereferenced by us.
pub type Device = cl_device_id;
type cl_context = *mut c_void;
type cl_command_queue = *mut c_void;
type cl_program = *mut c_void;
type cl_kernel = *mut c_void;
type cl_mem = *mut c_void;

const CL_SUCCESS: cl_int = 0;
const CL_DEVICE_NOT_FOUND: cl_int = -1;

pub const CL_DEVICE_TYPE_GPU: cl_device_type = 1 << 2;

pub const CL_DEVICE_MAX_COMPUTE_UNITS: cl_uint = 0x1002;
pub const CL_DEVICE_MAX_CLOCK_FREQUENCY: cl_uint = 0x100C;
pub const CL_DEVICE_MAX_MEM_ALLOC_SIZE: cl_uint = 0x1010;
pub const CL_DEVICE_GLOBAL_MEM_SIZE: cl_uint = 0x101F;
pub const CL_DEVICE_NAME: cl_uint = 0x102B;
pub const CL_DEVICE_VENDOR: cl_uint = 0x102C;
pub const CL_DRIVER_VERSION: cl_uint = 0x102D;
pub const CL_DEVICE_VERSION: cl_uint = 0x102F;
pub const CL_DEVICE_HOST_UNIFIED_MEMORY: cl_uint = 0x1035;

const CL_PROGRAM_BUILD_LOG: cl_uint = 0x1183;

pub const CL_MEM_WRITE_ONLY: cl_mem_flags = 1 << 1;
pub const CL_MEM_READ_ONLY: cl_mem_flags = 1 << 2;
const CL_MEM_COPY_HOST_PTR: cl_mem_flags = 1 << 5;
pub const CL_MEM_HOST_NO_ACCESS: cl_mem_flags = 1 << 9;

// --- entry-point signatures --------------------------------------------------

type FnGetPlatformIDs = unsafe extern "C" fn(cl_uint, *mut cl_platform_id, *mut cl_uint) -> cl_int;
type FnGetDeviceIDs = unsafe extern "C" fn(
    cl_platform_id,
    cl_device_type,
    cl_uint,
    *mut cl_device_id,
    *mut cl_uint,
) -> cl_int;
type FnGetDeviceInfo =
    unsafe extern "C" fn(cl_device_id, cl_uint, usize, *mut c_void, *mut usize) -> cl_int;
type FnCreateContext = unsafe extern "C" fn(
    *const isize,
    cl_uint,
    *const cl_device_id,
    *const c_void,
    *mut c_void,
    *mut cl_int,
) -> cl_context;
type FnCreateCommandQueue = unsafe extern "C" fn(
    cl_context,
    cl_device_id,
    cl_command_queue_properties,
    *mut cl_int,
) -> cl_command_queue;
type FnCreateCommandQueueWithProperties = unsafe extern "C" fn(
    cl_context,
    cl_device_id,
    *const cl_bitfield,
    *mut cl_int,
) -> cl_command_queue;
type FnCreateProgramWithSource = unsafe extern "C" fn(
    cl_context,
    cl_uint,
    *const *const c_char,
    *const usize,
    *mut cl_int,
) -> cl_program;
type FnBuildProgram = unsafe extern "C" fn(
    cl_program,
    cl_uint,
    *const cl_device_id,
    *const c_char,
    *const c_void,
    *mut c_void,
) -> cl_int;
type FnGetProgramBuildInfo = unsafe extern "C" fn(
    cl_program,
    cl_device_id,
    cl_uint,
    usize,
    *mut c_void,
    *mut usize,
) -> cl_int;
type FnCreateKernel = unsafe extern "C" fn(cl_program, *const c_char, *mut cl_int) -> cl_kernel;
type FnSetKernelArg = unsafe extern "C" fn(cl_kernel, cl_uint, usize, *const c_void) -> cl_int;
type FnCreateBuffer =
    unsafe extern "C" fn(cl_context, cl_mem_flags, usize, *mut c_void, *mut cl_int) -> cl_mem;
type FnEnqueueNDRangeKernel = unsafe extern "C" fn(
    cl_command_queue,
    cl_kernel,
    cl_uint,
    *const usize,
    *const usize,
    *const usize,
    cl_uint,
    *const c_void,
    *mut c_void,
) -> cl_int;
type FnEnqueueReadBuffer = unsafe extern "C" fn(
    cl_command_queue,
    cl_mem,
    cl_uint,
    usize,
    usize,
    *mut c_void,
    cl_uint,
    *const c_void,
    *mut c_void,
) -> cl_int;
type FnFinish = unsafe extern "C" fn(cl_command_queue) -> cl_int;
type FnRelease1 = unsafe extern "C" fn(*mut c_void) -> cl_int;

struct Inner {
    _lib: Library,
    get_platform_ids: FnGetPlatformIDs,
    get_device_ids: FnGetDeviceIDs,
    get_device_info: FnGetDeviceInfo,
    create_context: FnCreateContext,
    create_command_queue: Option<FnCreateCommandQueue>,
    create_command_queue_with_properties: Option<FnCreateCommandQueueWithProperties>,
    create_program_with_source: FnCreateProgramWithSource,
    build_program: FnBuildProgram,
    get_program_build_info: FnGetProgramBuildInfo,
    create_kernel: FnCreateKernel,
    set_kernel_arg: FnSetKernelArg,
    create_buffer: FnCreateBuffer,
    enqueue_ndrange: FnEnqueueNDRangeKernel,
    enqueue_read: FnEnqueueReadBuffer,
    finish: FnFinish,
    release_context: FnRelease1,
    release_queue: FnRelease1,
    release_program: FnRelease1,
    release_kernel: FnRelease1,
    release_mem: FnRelease1,
}

/// A loaded OpenCL ICD. Cheap to clone (an `Arc`); every handle keeps one so the
/// library outlives it.
#[derive(Clone)]
pub struct Cl(Arc<Inner>);

fn lib_names() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["OpenCL.dll"]
    }
    #[cfg(target_os = "macos")]
    {
        &["/System/Library/Frameworks/OpenCL.framework/OpenCL"]
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        &["libOpenCL.so.1", "libOpenCL.so"]
    }
}

fn check(code: cl_int, what: &str) -> Result<()> {
    if code == CL_SUCCESS {
        Ok(())
    } else {
        bail!("{what}: OpenCL error {code}")
    }
}

impl Cl {
    /// Load the ICD loader. `Err` (not a panic) if OpenCL is not installed.
    pub fn load() -> Result<Self> {
        let lib = lib_names()
            .iter()
            .find_map(|n| unsafe { Library::new(n) }.ok())
            .ok_or_else(|| anyhow!("OpenCL ICD loader not found (no GPU compute support)"))?;

        unsafe fn req<T: Copy>(lib: &Library, name: &str) -> Result<T> {
            let sym: Symbol<T> = unsafe { lib.get(name.as_bytes()) }.map_err(|_| {
                anyhow!("OpenCL: entry point {} is missing", &name[..name.len() - 1])
            })?;
            Ok(*sym)
        }
        unsafe fn opt<T: Copy>(lib: &Library, name: &str) -> Option<T> {
            unsafe { lib.get::<T>(name.as_bytes()) }.ok().map(|s| *s)
        }

        let inner = unsafe {
            Inner {
                get_platform_ids: req(&lib, "clGetPlatformIDs\0")?,
                get_device_ids: req(&lib, "clGetDeviceIDs\0")?,
                get_device_info: req(&lib, "clGetDeviceInfo\0")?,
                create_context: req(&lib, "clCreateContext\0")?,
                create_command_queue: opt(&lib, "clCreateCommandQueue\0"),
                create_command_queue_with_properties: opt(
                    &lib,
                    "clCreateCommandQueueWithProperties\0",
                ),
                create_program_with_source: req(&lib, "clCreateProgramWithSource\0")?,
                build_program: req(&lib, "clBuildProgram\0")?,
                get_program_build_info: req(&lib, "clGetProgramBuildInfo\0")?,
                create_kernel: req(&lib, "clCreateKernel\0")?,
                set_kernel_arg: req(&lib, "clSetKernelArg\0")?,
                create_buffer: req(&lib, "clCreateBuffer\0")?,
                enqueue_ndrange: req(&lib, "clEnqueueNDRangeKernel\0")?,
                enqueue_read: req(&lib, "clEnqueueReadBuffer\0")?,
                finish: req(&lib, "clFinish\0")?,
                release_context: req(&lib, "clReleaseContext\0")?,
                release_queue: req(&lib, "clReleaseCommandQueue\0")?,
                release_program: req(&lib, "clReleaseProgram\0")?,
                release_kernel: req(&lib, "clReleaseKernel\0")?,
                release_mem: req(&lib, "clReleaseMemObject\0")?,
                _lib: lib,
            }
        };
        if inner.create_command_queue.is_none()
            && inner.create_command_queue_with_properties.is_none()
        {
            bail!("OpenCL: no command-queue constructor exported");
        }
        Ok(Cl(Arc::new(inner)))
    }

    pub fn platform_ids(&self) -> Result<Vec<cl_platform_id>> {
        let mut n: cl_uint = 0;
        check(
            unsafe { (self.0.get_platform_ids)(0, ptr::null_mut(), &mut n) },
            "clGetPlatformIDs(count)",
        )?;
        let mut ids = vec![ptr::null_mut(); n as usize];
        if n > 0 {
            check(
                unsafe { (self.0.get_platform_ids)(n, ids.as_mut_ptr(), ptr::null_mut()) },
                "clGetPlatformIDs",
            )?;
        }
        Ok(ids)
    }

    pub fn device_ids(
        &self,
        platform: cl_platform_id,
        ty: cl_device_type,
    ) -> Result<Vec<cl_device_id>> {
        let mut n: cl_uint = 0;
        let rc = unsafe { (self.0.get_device_ids)(platform, ty, 0, ptr::null_mut(), &mut n) };
        if rc == CL_DEVICE_NOT_FOUND {
            return Ok(Vec::new());
        }
        check(rc, "clGetDeviceIDs(count)")?;
        let mut ids = vec![ptr::null_mut(); n as usize];
        if n > 0 {
            check(
                unsafe {
                    (self.0.get_device_ids)(platform, ty, n, ids.as_mut_ptr(), ptr::null_mut())
                },
                "clGetDeviceIDs",
            )?;
        }
        Ok(ids)
    }

    pub fn device_string(&self, device: cl_device_id, param: cl_uint) -> Result<String> {
        let mut size: usize = 0;
        check(
            unsafe { (self.0.get_device_info)(device, param, 0, ptr::null_mut(), &mut size) },
            "clGetDeviceInfo(size)",
        )?;
        let mut buf = vec![0u8; size];
        check(
            unsafe {
                (self.0.get_device_info)(
                    device,
                    param,
                    size,
                    buf.as_mut_ptr() as *mut c_void,
                    ptr::null_mut(),
                )
            },
            "clGetDeviceInfo",
        )?;
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Ok(String::from_utf8_lossy(&buf).trim().to_string())
    }

    fn device_scalar<T: Copy + Default>(&self, device: cl_device_id, param: cl_uint) -> Result<T> {
        let mut v = T::default();
        check(
            unsafe {
                (self.0.get_device_info)(
                    device,
                    param,
                    std::mem::size_of::<T>(),
                    &mut v as *mut T as *mut c_void,
                    ptr::null_mut(),
                )
            },
            "clGetDeviceInfo(scalar)",
        )?;
        Ok(v)
    }

    pub fn device_u32(&self, device: cl_device_id, param: cl_uint) -> Result<u32> {
        self.device_scalar::<cl_uint>(device, param)
    }

    pub fn device_u64(&self, device: cl_device_id, param: cl_uint) -> Result<u64> {
        self.device_scalar::<cl_ulong>(device, param)
    }

    pub fn context(&self, device: cl_device_id) -> Result<Context> {
        let mut err: cl_int = 0;
        let ctx = unsafe {
            (self.0.create_context)(
                ptr::null(),
                1,
                &device,
                ptr::null(),
                ptr::null_mut(),
                &mut err,
            )
        };
        check(err, "clCreateContext")?;
        if ctx.is_null() {
            bail!("clCreateContext returned null");
        }
        Ok(Context {
            cl: self.clone(),
            handle: ctx,
            device,
        })
    }
}

pub struct Context {
    cl: Cl,
    handle: cl_context,
    device: cl_device_id,
}

impl Context {
    pub fn queue(&self) -> Result<Queue> {
        let mut err: cl_int = 0;
        let q = if let Some(f) = self.cl.0.create_command_queue_with_properties {
            unsafe { f(self.handle, self.device, ptr::null(), &mut err) }
        } else {
            let f = self.cl.0.create_command_queue.unwrap();
            unsafe { f(self.handle, self.device, 0, &mut err) }
        };
        check(err, "clCreateCommandQueue")?;
        if q.is_null() {
            bail!("clCreateCommandQueue returned null");
        }
        Ok(Queue {
            cl: self.cl.clone(),
            handle: q,
        })
    }

    pub fn program(&self, source: &str) -> Result<Program> {
        let src = CString::new(source)?;
        let strings = [src.as_ptr()];
        let lengths = [source.len()];
        let mut err: cl_int = 0;
        let prog = unsafe {
            (self.cl.0.create_program_with_source)(
                self.handle,
                1,
                strings.as_ptr(),
                lengths.as_ptr(),
                &mut err,
            )
        };
        check(err, "clCreateProgramWithSource")?;

        let rc = unsafe {
            (self.cl.0.build_program)(
                prog,
                1,
                &self.device,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
            )
        };
        if rc != CL_SUCCESS {
            let log = self.build_log(prog).unwrap_or_default();
            unsafe { (self.cl.0.release_program)(prog) };
            bail!("OpenCL kernel build failed (error {rc}): {log}");
        }
        Ok(Program {
            cl: self.cl.clone(),
            handle: prog,
        })
    }

    fn build_log(&self, prog: cl_program) -> Result<String> {
        let mut size: usize = 0;
        check(
            unsafe {
                (self.cl.0.get_program_build_info)(
                    prog,
                    self.device,
                    CL_PROGRAM_BUILD_LOG,
                    0,
                    ptr::null_mut(),
                    &mut size,
                )
            },
            "clGetProgramBuildInfo(size)",
        )?;
        let mut buf = vec![0u8; size];
        check(
            unsafe {
                (self.cl.0.get_program_build_info)(
                    prog,
                    self.device,
                    CL_PROGRAM_BUILD_LOG,
                    size,
                    buf.as_mut_ptr() as *mut c_void,
                    ptr::null_mut(),
                )
            },
            "clGetProgramBuildInfo",
        )?;
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Ok(String::from_utf8_lossy(&buf).trim().to_string())
    }

    pub fn buffer(&self, flags: cl_mem_flags, bytes: usize) -> Result<Mem> {
        self.make_buffer(flags, bytes, ptr::null_mut())
    }

    /// A buffer initialised with a copy of `data` (`CL_MEM_COPY_HOST_PTR`).
    pub fn buffer_copy(&self, flags: cl_mem_flags, data: &[u8]) -> Result<Mem> {
        self.make_buffer(
            flags | CL_MEM_COPY_HOST_PTR,
            data.len(),
            data.as_ptr() as *mut c_void,
        )
    }

    fn make_buffer(&self, flags: cl_mem_flags, bytes: usize, host_ptr: *mut c_void) -> Result<Mem> {
        let mut err: cl_int = 0;
        let m = unsafe { (self.cl.0.create_buffer)(self.handle, flags, bytes, host_ptr, &mut err) };
        check(err, "clCreateBuffer")?;
        if m.is_null() {
            bail!("clCreateBuffer returned null");
        }
        Ok(Mem {
            cl: self.cl.clone(),
            handle: m,
            bytes,
        })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { (self.cl.0.release_context)(self.handle) };
    }
}

pub struct Program {
    cl: Cl,
    handle: cl_program,
}

impl Program {
    pub fn kernel(&self, name: &str) -> Result<Kernel> {
        let cname = CString::new(name)?;
        let mut err: cl_int = 0;
        let k = unsafe { (self.cl.0.create_kernel)(self.handle, cname.as_ptr(), &mut err) };
        check(err, "clCreateKernel")?;
        if k.is_null() {
            bail!("clCreateKernel({name}) returned null");
        }
        Ok(Kernel {
            cl: self.cl.clone(),
            handle: k,
        })
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        unsafe { (self.cl.0.release_program)(self.handle) };
    }
}

pub struct Kernel {
    cl: Cl,
    handle: cl_kernel,
}

impl Kernel {
    pub fn set_mem(&self, index: cl_uint, mem: &Mem) -> Result<()> {
        check(
            unsafe {
                (self.cl.0.set_kernel_arg)(
                    self.handle,
                    index,
                    std::mem::size_of::<cl_mem>(),
                    &mem.handle as *const cl_mem as *const c_void,
                )
            },
            "clSetKernelArg(mem)",
        )
    }

    pub fn set_u32(&self, index: cl_uint, value: u32) -> Result<()> {
        check(
            unsafe {
                (self.cl.0.set_kernel_arg)(
                    self.handle,
                    index,
                    std::mem::size_of::<u32>(),
                    &value as *const u32 as *const c_void,
                )
            },
            "clSetKernelArg(u32)",
        )
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        unsafe { (self.cl.0.release_kernel)(self.handle) };
    }
}

pub struct Mem {
    cl: Cl,
    handle: cl_mem,
    bytes: usize,
}

impl Drop for Mem {
    fn drop(&mut self) {
        unsafe { (self.cl.0.release_mem)(self.handle) };
    }
}

pub struct Queue {
    cl: Cl,
    handle: cl_command_queue,
}

impl Queue {
    /// Enqueue a 1-D kernel. `local` of 0 lets the driver choose.
    pub fn run_1d(&self, kernel: &Kernel, global: usize, local: usize) -> Result<()> {
        let g = [global];
        let l = [local];
        check(
            unsafe {
                (self.cl.0.enqueue_ndrange)(
                    self.handle,
                    kernel.handle,
                    1,
                    ptr::null(),
                    g.as_ptr(),
                    if local == 0 { ptr::null() } else { l.as_ptr() },
                    0,
                    ptr::null(),
                    ptr::null_mut(),
                )
            },
            "clEnqueueNDRangeKernel",
        )
    }

    pub fn read(&self, mem: &Mem, out: &mut [u8]) -> Result<()> {
        check(
            unsafe {
                (self.cl.0.enqueue_read)(
                    self.handle,
                    mem.handle,
                    1, // blocking
                    0,
                    out.len().min(mem.bytes),
                    out.as_mut_ptr() as *mut c_void,
                    0,
                    ptr::null(),
                    ptr::null_mut(),
                )
            },
            "clEnqueueReadBuffer",
        )
    }

    pub fn finish(&self) -> Result<()> {
        check(unsafe { (self.cl.0.finish)(self.handle) }, "clFinish")
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        unsafe { (self.cl.0.release_queue)(self.handle) };
    }
}
