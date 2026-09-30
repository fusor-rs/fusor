if (typeof document === 'undefined') throw Error('UI import executed in a worker');
window.workerFixtureUiLoads = (window.workerFixtureUiLoads || 0) + 1;
export function onMount() {}
