//! The Vulkan pipeline cache of the GS capture kernels: the shared implementation lives in `crate::pipecache` (the level pipelines
//! use it too); the kernels' file name (`gpu_kernels_<key>.bin`) and behaviour are unchanged.

pub(crate) use crate::pipecache::PipeCache;
