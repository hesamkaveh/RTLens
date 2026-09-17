import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { classifyText, RTL_RATIO } = require('../integrations/antigravity/client.cjs');
const { getClientScript } = require('../scripts/inject-antigravity.cjs');

test('text without RTL letters is left to the browser', () => {
    assert.equal(classifyText('Hello world', RTL_RATIO), null);
    assert.equal(classifyText('', RTL_RATIO), null);
});

test('Persian prose is RTL even when it starts with a Latin word', () => {
    assert.equal(classifyText('RTLens یک ابزار برای نمایش متن است', RTL_RATIO), 'rtl');
});

test('a single Persian word in English prose stays LTR', () => {
    assert.equal(classifyText('The Persian word سلام means hello in this sentence', RTL_RATIO), 'ltr');
});

test('client script is valid JavaScript', () => {
    assert.doesNotThrow(() => new Function(getClientScript()));
});
