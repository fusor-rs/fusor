use fusor_worker::{task as background, TaskResult};
#[background]
pub fn twice(value: i32) -> TaskResult<i32> { Ok(value * 4) }
#[background]
#[cfg(any())]
pub fn disabled(_: ()) -> TaskResult<()> { unimplemented!() }

pub mod first { pub struct Client(pub(crate) u32); }
pub mod second { pub struct Client(pub(crate) u32); }
#[fusor_worker::worker]
impl first::Client {
    pub fn new(input: u32) -> TaskResult<Self> { Ok(Self(input)) }
    pub fn add(&mut self, input: u32) -> TaskResult<u32> { self.0 += input; Ok(self.0) }
}
#[fusor_worker::worker]
impl second::Client {
    pub fn new(input: u32) -> TaskResult<Self> { Ok(Self(input)) }
    pub fn add(&mut self, input: u32) -> TaskResult<u32> { self.0 += input; Ok(self.0) }
}
