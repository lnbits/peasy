/* Metadata is read from GitHub's published release, not an unauthenticated R2 manifest. */
(function () {
  'use strict';
  const releases = 'https://github.com/lnbits/peasy/releases';
  const api = 'https://api.github.com/repos/lnbits/peasy/releases/latest';

  function parseRelease(release) {
    if (!release || release.draft !== false || release.prerelease !== false ||
        !/^v[A-Za-z0-9._-]+$/.test(release.tag_name) || typeof release.body !== 'string') {
      throw new Error('Not a published stable Peasy release');
    }
    const markers = [...release.body.matchAll(/<!-- peasy-iso-downloads:(.*?) -->/g)];
    if (markers.length !== 1) throw new Error('No unambiguous full-ISO metadata');
    const manifest = JSON.parse(markers[0][1]);
    if (manifest.schema !== 1 || manifest.tag !== release.tag_name ||
        !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(manifest.commit) ||
        !release.body.includes(`<!-- peasy-iso-release:${manifest.commit} -->`) ||
        !Array.isArray(manifest.images) || manifest.images.length !== 2 || !Array.isArray(release.assets)) {
      throw new Error('Invalid release metadata');
    }
    const images = ['gnome', 'plasma'].map(desktop => {
      const matches = manifest.images.filter(item => item && item.desktop === desktop);
      if (matches.length !== 1) throw new Error('Missing or duplicate desktop');
      const item = matches[0];
      const name = `peasy-nixos-${manifest.tag}-${desktop}-x86_64.iso`;
      const url = `https://downloads.askpeasy.com/peasy/releases/${manifest.tag}/${manifest.commit}/${name}`;
      const checksumURL = `${releases}/download/${manifest.tag}/${name}.sha256`;
      const assets = release.assets.filter(asset => asset.name === `${name}.sha256`);
      if (item.name !== name || item.url !== url || !Number.isSafeInteger(item.size) || item.size <= 0 ||
          !/^[0-9a-f]{64}$/.test(item.sha256) || assets.length !== 1 ||
          assets[0].state !== 'uploaded' || assets[0].browser_download_url !== checksumURL) {
        throw new Error('Invalid ISO or checksum link');
      }
      return {...item, checksumURL};
    });
    return {tag: manifest.tag, url: `${releases}/tag/${manifest.tag}`, images};
  }

  async function loadDownloads(doc, fetchRelease = fetch) {
    const status = doc.getElementById('download-status');
    if (!status) return;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 8000);
    try {
      const response = await fetchRelease(api, {
        signal: controller.signal, credentials: 'omit', headers: {Accept: 'application/vnd.github+json'},
      });
      if (!response.ok) throw new Error('Release lookup unavailable');
      const release = parseRelease(await response.json());
      for (const item of release.images) {
        const button = doc.getElementById(`download-${item.desktop}`);
        button.href = item.url;
        button.textContent = `Download ${item.desktop === 'gnome' ? 'GNOME' : 'Plasma'} ISO ↓`;
        doc.getElementById(`checksum-${item.desktop}`).href = item.checksumURL;
        doc.getElementById(`download-${item.desktop}-meta`).textContent =
          `64-bit Intel / AMD · ${(item.size / 1024 ** 3).toFixed(2)} GiB · ${release.tag}`;
      }
      doc.getElementById('download-release').href = release.url;
      status.textContent = `Latest release: ${release.tag}. Full images, ready to download — no parts to join.`;
    } catch (_) {
      status.textContent = 'Check GitHub Releases for the latest available images and checksums.';
    } finally {
      clearTimeout(timer);
    }
  }

  if (typeof module !== 'undefined' && module.exports) module.exports = {parseRelease, loadDownloads};
  if (typeof document !== 'undefined') loadDownloads(document);
})();
