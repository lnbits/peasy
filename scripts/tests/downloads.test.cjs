const {test} = require('node:test');
const assert = require('node:assert/strict');
const {parseRelease, loadDownloads} = require('../../assets/downloads.js');
const {readFileSync} = require('node:fs');

function fixture() {
  const tag = 'v1.2.3', commit = 'a'.repeat(40);
  const manifest = {schema: 1, tag, commit, images: ['gnome', 'plasma'].map(desktop => {
    const name = `peasy-nixos-${tag}-${desktop}-x86_64.iso`;
    return {desktop, name, size: 6 * 1024 ** 3, sha256: 'b'.repeat(64),
      url: `https://downloads.askpeasy.com/peasy/releases/${tag}/${commit}/${name}`};
  })};
  const release = {tag_name: tag, draft: false, prerelease: false,
    assets: manifest.images.map(item => ({name: `${item.name}.sha256`, state: 'uploaded',
      browser_download_url: `https://github.com/lnbits/peasy/releases/download/${tag}/${item.name}.sha256`}))};
  const encode = () => ({...release, body:
    `<!-- peasy-iso-release:${commit} -->\n<!-- peasy-iso-downloads:${JSON.stringify(manifest)} -->`});
  return {manifest, release, encode};
}

test('published full-ISO release provides both exact download and checksum links', () => {
  const parsed = parseRelease(fixture().encode());
  assert.equal(parsed.tag, 'v1.2.3');
  assert.equal(parsed.images.length, 2);
  assert.ok(parsed.images[0].checksumURL.endsWith('.iso.sha256'));
});

test('reject untrusted, malformed, incomplete or unpublished metadata', () => {
  const mutations = [
    f => { f.release.draft = true; }, f => { f.release.prerelease = true; },
    f => { f.release.tag_name = 'v2'; }, f => { f.manifest.schema = 2; },
    f => { f.manifest.commit = 'short'; }, f => { f.manifest.images.pop(); },
    f => { f.manifest.images[1] = f.manifest.images[0]; },
    f => { f.manifest.images[0].size = -1; }, f => { f.manifest.images[0].sha256 = 'bad'; },
    f => { f.manifest.images[0].name = '../image.iso'; },
    f => { f.manifest.images[0].url = 'javascript:alert(1)'; },
    f => { f.manifest.images[0].url = f.manifest.images[0].url.replace('askpeasy.com', 'evil.example'); },
    f => { f.release.assets[0].browser_download_url = 'https://evil.example/file'; },
    f => { f.release.assets[0].state = 'new'; }, f => { f.release.assets.pop(); },
  ];
  for (const mutate of mutations) {
    const f = fixture(); mutate(f);
    assert.throws(() => parseRelease(f.encode()));
  }
  assert.throws(() => parseRelease({...fixture().encode(), body: 'Legacy split release'}));
  const f = fixture().encode();
  assert.throws(() => parseRelease({...f, body: f.body + f.body}));
});

function documentStub() {
  const elements = new Map();
  return {getElementById(id) {
    if (!elements.has(id)) elements.set(id, {textContent: '', href: 'https://github.com/lnbits/peasy/releases/latest'});
    return elements.get(id);
  }};
}

test('UI resolves both cards without HTML injection', async () => {
  const doc = documentStub();
  await loadDownloads(doc, async (url, options) => {
    assert.equal(options.credentials, 'omit');
    assert.equal(url, 'https://api.github.com/repos/lnbits/peasy/releases/latest');
    return {ok: true, json: async () => fixture().encode()};
  });
  assert.match(doc.getElementById('download-gnome').href, /^https:\/\/downloads.askpeasy.com\//);
  assert.equal(doc.getElementById('download-plasma').textContent, 'Download Plasma ISO ↓');
  assert.match(doc.getElementById('download-plasma-meta').textContent, /6.00 GiB/);
});

test('network, rate limit and invalid metadata preserve functional release-page fallback', async () => {
  for (const fetch of [async () => {throw new Error('offline');}, async () => ({ok: false}),
    async () => ({ok: true, json: async () => ({})})]) {
    const doc = documentStub();
    await loadDownloads(doc, fetch);
    assert.equal(doc.getElementById('download-gnome').href, 'https://github.com/lnbits/peasy/releases/latest');
    assert.match(doc.getElementById('download-status').textContent, /Check GitHub/);
  }
});

test('downloads is the third major section; no-JS links are present', () => {
  const html = readFileSync(new URL('../../index.html', `file://${__filename}`), 'utf8');
  const sections = [...html.matchAll(/<(?:header|section|footer)\b[^>]*>/g)].map(match => match[0]);
  assert.match(sections[0], /hero/);
  assert.match(sections[1], /id="why"/);
  assert.match(sections[2], /id="downloads"/);
  for (const desktop of ['gnome', 'plasma']) {
    assert.ok(html.includes(`id="download-${desktop}" href="https://github.com/lnbits/peasy/releases/latest"`));
  }
});
