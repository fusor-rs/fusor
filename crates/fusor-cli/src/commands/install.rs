use crate::{context::Context, error::Result, toolchain, workspace::Project};

pub(crate) fn run(cx: &Context, project: &Project) -> Result {
    toolchain::prepare(cx, project, true, false)?;
    crate::pipeline::workers::prepare(cx, project)
}
