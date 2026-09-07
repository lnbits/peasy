const {test} = require('node:test');
const assert = require('node:assert/strict');
const {parseRelease, loadDownloads} = require('../../assets/downloads.js');
const {readFileSync} = require('node:fs');

function fixture(desktops = ['gnome']) {
  const tag = 'v1.2.3', commit = 'a'.repeat(40);
  const manifest = {schema: 1, tag, commit, images: desktops.map(desktop => {
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

test('single-ISO release provides exact download and checksum links', () => {
  const parsed = parseRelease(fixture().encode());
  assert.equal(parsed.tag, 'v1.2.3');
  assert.equal(parsed.images.length, 1);
  assert.ok(parsed.images[0].checksumURL.endsWith('.iso.sha256'));
});

test('previous dual-image releases remain readable during migration', () => {
  const parsed = parseRelease(fixture(['gnome', 'plasma']).encode());
  assert.deepEqual(parsed.images.map(item => item.desktop), ['gnome', 'plasma']);
  assert.throws(() => parseRelease(fixture(['plasma']).encode()));
  assert.throws(() => parseRelease(fixture(['gnome', 'xfce']).encode()));
  assert.throws(() => parseRelease(fixture(['gnome', 'plasma', 'xfce']).encode()));
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
    assert.ok(!id.includes('plasma'), 'The website has only one ISO card');
    if (!elements.has(id)) elements.set(id, {textContent: '',
      ...(id === 'download-gnome' ? {} : {href: 'https://github.com/lnbits/peasy/releases/latest'}),
      setAttribute(name, value) { this[name] = value; },
      removeAttribute(name) { delete this[name]; },
    });
    return elements.get(id);
  }};
}

test('UI resolves the single card for new and previous releases without HTML injection', async () => {
  for (const desktops of [['gnome'], ['gnome', 'plasma']]) {
  const doc = documentStub();
  await loadDownloads(doc, async (url, options) => {
    assert.equal(options.credentials, 'omit');
    assert.equal(url, 'https://api.github.com/repos/lnbits/peasy/releases/latest');
    return {ok: true, json: async () => fixture(desktops).encode()};
  });
  assert.match(doc.getElementById('download-gnome').href, /^https:\/\/downloads.askpeasy.com\//);
  assert.equal(doc.getElementById('download-gnome').textContent, 'Download Peasy ISO ↓');
  assert.equal(doc.getElementById('download-gnome')['aria-disabled'], undefined);
  assert.equal(doc.getElementById('download-gnome').download, 'peasy-nixos-v1.2.3-gnome-x86_64.iso');
  assert.equal(doc.getElementById('checksum-gnome').href, 'https://github.com/lnbits/peasy/releases/tag/v1.2.3');
  assert.match(doc.getElementById('download-gnome-meta').textContent, /6.00 GiB/);
  }
});

test('network, rate limit and invalid metadata disable download but leave release checksums available', async () => {
  for (const fetch of [async () => {throw new Error('offline');}, async () => ({ok: false}),
    async () => ({ok: true, json: async () => ({})})]) {
    const doc = documentStub();
    await loadDownloads(doc, fetch);
    assert.equal(doc.getElementById('download-gnome').href, undefined);
    assert.equal(doc.getElementById('download-gnome')['aria-disabled'], 'true');
    assert.equal(doc.getElementById('download-gnome').textContent, 'Download Peasy ISO ↓');
    assert.equal(doc.getElementById('checksum-gnome').href, 'https://github.com/lnbits/peasy/releases/latest');
    assert.match(doc.getElementById('download-status').textContent, /unavailable right now/);
  }
});

test('no published release disables downloading, including after an earlier successful lookup', async () => {
  const doc = documentStub();
  await loadDownloads(doc, async () => ({ok: true, json: async () => fixture().encode()}));
  assert.ok(doc.getElementById('download-gnome').href.endsWith('.iso'));
  await loadDownloads(doc, async () => ({ok: false, status: 404}));
  assert.equal(doc.getElementById('download-gnome').href, undefined);
  assert.equal(doc.getElementById('download-gnome').download, undefined);
  assert.equal(doc.getElementById('download-gnome')['aria-disabled'], 'true');
  assert.match(doc.getElementById('download-status').textContent, /No published ISO release/);
});

test('downloads is the third major section; no-JS links are present', () => {
  const html = readFileSync(new URL('../../index.html', `file://${__filename}`), 'utf8');
  const sections = [...html.matchAll(/<(?:header|section|footer)\b[^>]*>/g)].map(match => match[0]);
  assert.match(sections[0], /hero/);
  assert.match(sections[1], /id="why"/);
  assert.match(sections[2], /id="downloads"/);
  const button = html.match(/<a\b[^>]*id="download-gnome"[^>]*>/)[0];
  assert.ok(!button.includes('href='));
  assert.ok(button.includes('aria-disabled="true"'));
  assert.ok(html.includes('id="checksum-gnome" href="https://github.com/lnbits/peasy/releases/latest"'));
  assert.ok(html.includes('<noscript>'));
  assert.ok(!html.includes('id="download-plasma"'));
  assert.ok(html.includes('XFCE installation requires an Internet connection'));
});
