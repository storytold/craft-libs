// Test-only browser fault injection; no app runtime depends on this module.
let originals;
let created, revoked, clicked, scheduled, lastName, lastMime, timerBeforeClick;
export function setup(mode) {
    created = 0; revoked = 0; clicked = 0; scheduled = 0;
    lastName = ''; lastMime = ''; timerBeforeClick = false;
    originals = {
        create: URL.createObjectURL, revoke: URL.revokeObjectURL,
        click: HTMLAnchorElement.prototype.click,
        append: Node.prototype.appendChild, timer: window.setTimeout,
        element: Document.prototype.createElement,
        attribute: Element.prototype.setAttribute,
    };
    URL.createObjectURL = function(blob) { created++; lastMime = blob.type; return originals.create.call(URL, blob); };
    URL.revokeObjectURL = function(url) { revoked++; return originals.revoke.call(URL, url); };
    HTMLAnchorElement.prototype.click = function() {
        clicked++; lastName = this.download; timerBeforeClick = scheduled > 0;
    };
    window.setTimeout = function(...args) {
        if (mode === 'timer-error') throw new Error('timer failed');
        scheduled++;
        return originals.timer.apply(window, args);
    };
    if (mode === 'attach-error') Node.prototype.appendChild = function() { throw new Error('attachment failed'); };
    if (mode === 'element-error') Document.prototype.createElement = function() { throw new Error('element failed'); };
    if (mode === 'style-error') Element.prototype.setAttribute = function() { throw new Error('style failed'); };
}
export function restore() {
    URL.createObjectURL = originals.create; URL.revokeObjectURL = originals.revoke;
    HTMLAnchorElement.prototype.click = originals.click;
    Node.prototype.appendChild = originals.append; window.setTimeout = originals.timer;
    Document.prototype.createElement = originals.element;
    Element.prototype.setAttribute = originals.attribute;
}
export function created_count() { return created; }
export function revoked_count() { return revoked; }
export function clicked_count() { return clicked; }
export function anchor_count() { return document.querySelectorAll('a[download]').length; }
export function last_name() { return lastName; }
export function last_mime() { return lastMime; }
export function timer_before_click() { return timerBeforeClick; }
export function wait_for_cleanup() { return new Promise(resolve => originals.timer.call(window, resolve, 50)); }
