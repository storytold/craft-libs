// Test-only browser instrumentation; no app runtime depends on this module.
let originals;
let revoked, clicked, scheduled, lastName, lastMime, timerBeforeClick;
export function setup() {
    revoked = 0; clicked = 0; scheduled = 0;
    lastName = ''; lastMime = ''; timerBeforeClick = false;
    originals = {
        create: URL.createObjectURL, revoke: URL.revokeObjectURL,
        click: HTMLAnchorElement.prototype.click, timer: window.setTimeout,
    };
    URL.createObjectURL = function(blob) { lastMime = blob.type; return originals.create.call(URL, blob); };
    URL.revokeObjectURL = function(url) { revoked++; return originals.revoke.call(URL, url); };
    HTMLAnchorElement.prototype.click = function() {
        clicked++; lastName = this.download; timerBeforeClick = scheduled > 0;
    };
    window.setTimeout = function(...args) {
        scheduled++;
        return originals.timer.apply(window, args);
    };
}
export function restore() {
    URL.createObjectURL = originals.create; URL.revokeObjectURL = originals.revoke;
    HTMLAnchorElement.prototype.click = originals.click; window.setTimeout = originals.timer;
}
export function revoked_count() { return revoked; }
export function clicked_count() { return clicked; }
export function anchor_count() { return document.querySelectorAll('a[download]').length; }
export function last_name() { return lastName; }
export function last_mime() { return lastMime; }
export function timer_before_click() { return timerBeforeClick; }
export function wait_for_cleanup() { return new Promise(resolve => originals.timer.call(window, resolve, 50)); }
