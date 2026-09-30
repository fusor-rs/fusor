use fusor_worker::{JobError, TaskResult};

#[fusor_worker::task]
pub fn parse_count(text: String) -> TaskResult<u32, String> {
    text.parse::<u32>()
        .map_err(|error| JobError::Application(error.to_string()))
}
