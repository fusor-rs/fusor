export function setTextIfChanged(node, value) {
    if (node.data !== value) node.data = value;
}

export function setIntegerTextIfChanged(node, number) {
    const value = '' + number;
    setTextIfChanged(node, value);
}

export function setIntegerAttribute(node, name, number) {
    node.setAttribute(name, '' + number);
}

export function serverRows(container, name, encoded, count, complete) {
    const keys = count ? encoded.split('\n') : [];
    const rows = new Array(count);
    let node = container.firstElementChild;
    for (let index = 0; index < count; index++) {
        if (!node) throw 'missing native row';
        if (node.getAttribute(name) !== keys[index]) throw 'native row key mismatch';
        rows[index] = node;
        node = node.nextElementSibling;
    }
    if (complete ? node : !node) {
        throw complete ? 'unexpected native row' : 'missing native row';
    }
    return rows;
}

const listeners = [];
let removal;

export function listenBundleOk(nodes, index, name, dispatch, slot, generation) {
    try {
        return listenOk(nodes[index], name, dispatch, slot, generation);
    } catch (error) {
        removal = error;
        return false;
    }
}

export function unlistenBundleOk(nodes, index, name, slot) {
    try {
        return unlistenOk(nodes[index], name, slot);
    } catch (error) {
        removal = error;
        return false;
    }
}

export function unlistenOk(target, name, slot) {
    try {
        const listener = listeners[slot];
        listeners[slot] = undefined;
        target.removeEventListener(name, listener);
        return true;
    } catch (error) {
        removal = error;
        return false;
    }
}

export function insertBeforeOk(parent, node, anchor) {
    try {
        parent.insertBefore(node, anchor);
        return true;
    } catch (error) {
        removal = error;
        return false;
    }
}

export function takeRemovalFailure() {
    const error = removal;
    removal = undefined;
    return error;
}

export function listenOk(target, name, dispatch, slot, generation) {
    try {
        const listener = event => dispatch(slot, generation, event);
        target.addEventListener(name, listener);
        listeners[slot] = listener;
        return true;
    } catch (error) {
        removal = error;
        return false;
    }
}
