// RTLens RTL support for the Antigravity chat UI.
//
// Injected into Antigravity's window over the Chrome DevTools Protocol, both by
// the RTLens app (src-tauri/src/antigravity.rs, via include_str!) and by the dev
// CLI (scripts/inject-antigravity.cjs). Evaluating it again replaces the previous
// copy; `window.__rtlens.teardown()` removes it. Required from Node (no `window`)
// it only exports the pure helpers for tests.
(function () {
    'use strict';

    // Fraction of strong letters that must be RTL for a block to be laid out RTL.
    // First-strong-character detection gets "RTLens یک ابزار است" wrong; a ratio
    // doesn't, and a single Persian word in an English sentence stays LTR.
    const RTL_RATIO = 0.4;

    // Returns 'rtl', 'ltr' (mixed text that should be forced LTR), or null (no RTL
    // letters at all — leave it to the browser). Pure, so it can be unit-tested.
    function classifyText(text, ratio) {
        if (!text) return null;
        const rtl = (text.match(/[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFF]/g) || []).length;
        if (rtl === 0) return null;
        const ltr = (text.match(/[A-Za-z\u00C0-\u024F\u0370-\u03FF\u0400-\u04FF]/g) || []).length;
        return rtl / (rtl + ltr) >= ratio ? 'rtl' : 'ltr';
    }

    const RTL_CSS = `
    /* Baseline: every text block and editor picks its direction from its own
       content, per paragraph/line, before any script runs. */
    :is(p, li, blockquote, h1, h2, h3, h4, h5, h6, td, th, textarea, [contenteditable="true"], [data-quotable="true"]):not(.monaco-editor *) {
        unicode-bidi: plaintext !important;
        text-align: start !important;
    }

    /* Script-refined direction (ratio based) wins over first-strong detection. */
    [data-rtlens-dir="rtl"] {
        direction: rtl !important;
        unicode-bidi: isolate !important;
        text-align: right !important;
    }
    [data-rtlens-dir="ltr"] {
        direction: ltr !important;
        unicode-bidi: isolate !important;
        text-align: left !important;
    }

    :is(ul, ol)[data-rtlens-dir="rtl"] {
        padding-right: 1.75rem !important;
        padding-left: 0 !important;
        text-align: right !important;
    }

    :is(p, blockquote, h1, h2, h3, h4, h5, h6)[data-rtlens-dir="rtl"] {
        line-height: 1.75 !important;
    }

    [data-rtlens-dir="rtl"]:not(ul, ol) {
        font-family: -apple-system, BlinkMacSystemFont, "Vazirmatn", "Shabnam", "Sahel", "SF Arabic", "Geeza Pro", "Segoe UI", Roboto, sans-serif !important;
    }

    [data-testid="user-input-step"] [data-quotable="true"][data-rtlens-dir="rtl"],
    .artifact-card [data-rtlens-dir="rtl"] {
        display: block !important;
        width: 100% !important;
    }
    .artifact-card[data-rtlens-card="true"] {
        align-items: stretch !important;
    }

    /* Code stays LTR, including inline code inside RTL prose. */
    :is(pre, .monaco-editor) {
        direction: ltr !important;
        unicode-bidi: isolate !important;
        text-align: left !important;
    }
    [data-rtlens-dir="rtl"] code {
        direction: ltr !important;
        unicode-bidi: isolate !important;
    }
    `;

    if (typeof window === 'undefined') {
        if (typeof module !== 'undefined') module.exports = { classifyText, RTL_CSS, RTL_RATIO };
        return;
    }

    if (window.__rtlens && window.__rtlens.teardown) window.__rtlens.teardown();

    const BLOCKS = [
        'p', 'li', 'blockquote', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
        '[data-testid="user-input-step"] [data-quotable="true"]',
        '[aria-label="User message"] [data-quotable="true"]',
        '.artifact-card span',
        '[data-testid="conversation-row-sidebar"] .truncate',
        '[data-testid="conversation-list-item"] .truncate',
    ].join(', ');
    // Sidebar rows are usually buttons, so they're matched explicitly above and
    // only generic controls are skipped here.
    const SKIP = 'pre, code, .monaco-editor, script, style, svg, textarea, [contenteditable="true"], [data-testid="worked-for-collapsible"]';
    const SIDEBAR = '[data-testid="conversation-row-sidebar"], [data-testid="conversation-list-item"]';

    // Text of a block without its code, so identifiers don't skew the ratio.
    function proseText(el) {
        let out = '';
        const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        for (let n = walker.nextNode(); n; n = walker.nextNode()) {
            if (!n.parentElement.closest('pre, code')) out += n.data;
        }
        return out;
    }

    function setDir(el, dir) {
        if (dir) {
            if (el.getAttribute('data-rtlens-dir') !== dir) el.setAttribute('data-rtlens-dir', dir);
        } else if (el.hasAttribute('data-rtlens-dir')) {
            el.removeAttribute('data-rtlens-dir');
        }
    }

    function processList(list) {
        const items = Array.from(list.children).filter((c) => c.tagName === 'LI');
        const rtl = items.filter((li) => li.getAttribute('data-rtlens-dir') === 'rtl').length;
        setDir(list, items.length > 0 && rtl * 2 >= items.length ? 'rtl' : null);
    }

    function processBlock(el) {
        if (!el.isConnected || el.closest(SKIP)) return;
        if (el.closest('button, [role="button"]') && !el.closest(SIDEBAR)) return;

        setDir(el, classifyText(proseText(el), RTL_RATIO));

        if (el.tagName === 'LI' && el.parentElement && /^(UL|OL)$/.test(el.parentElement.tagName)) {
            processList(el.parentElement);
        }
        const card = el.closest('.artifact-card');
        if (card) {
            if (card.querySelector('[data-rtlens-dir="rtl"]')) card.setAttribute('data-rtlens-card', 'true');
            else card.removeAttribute('data-rtlens-card');
        }
    }

    let dirty = new Set();
    let flushTimer = null;

    function markClosest(node) {
        const el = node.nodeType === 1 ? node : node.parentElement;
        const own = el && el.closest(BLOCKS);
        if (own) dirty.add(own);
    }

    function markSubtree(node) {
        markClosest(node);
        if (node.nodeType === 1) node.querySelectorAll(BLOCKS).forEach((b) => dirty.add(b));
    }

    function flush() {
        flushTimer = null;
        const batch = dirty;
        dirty = new Set();
        batch.forEach(processBlock);
    }

    function schedule() {
        if (!flushTimer && dirty.size) flushTimer = setTimeout(flush, 50);
    }

    let style = null;
    let observer = null;

    function start() {
        style = document.createElement('style');
        style.id = 'rtlens-smart-rtl-style';
        style.textContent = RTL_CSS;
        (document.head || document.documentElement).appendChild(style);

        observer = new MutationObserver((mutations) => {
            for (const m of mutations) {
                // The target only needs its own enclosing block re-checked; only
                // newly added nodes are searched for blocks inside them.
                markClosest(m.target);
                if (m.type === 'childList') m.addedNodes.forEach(markSubtree);
            }
            schedule();
        });
        observer.observe(document.documentElement, { childList: true, subtree: true, characterData: true });

        markSubtree(document.documentElement);
        flush();
    }

    function onReady() {
        document.removeEventListener('DOMContentLoaded', onReady);
        start();
    }

    window.__rtlens = {
        teardown() {
            document.removeEventListener('DOMContentLoaded', onReady);
            if (observer) observer.disconnect();
            if (flushTimer) clearTimeout(flushTimer);
            if (style) style.remove();
            document.querySelectorAll('[data-rtlens-dir], [data-rtlens-card]').forEach((el) => {
                el.removeAttribute('data-rtlens-dir');
                el.removeAttribute('data-rtlens-card');
            });
            delete window.__rtlens;
        },
    };

    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', onReady);
    else start();
})();
