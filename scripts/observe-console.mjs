export async function observeConsoleErrors(page) {
  const errors = [];
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.addInitScript(() => {
    const report = console.error;
    console.error = (...values) => report(...values.map(String));
  });
  return errors;
}
