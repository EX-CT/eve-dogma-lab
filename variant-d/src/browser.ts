/** Browser helper: fetch + gunzip (DecompressionStream) + sha256 (SubtleCrypto). */
import { Dataset } from './core/dataset.js';
export * from './index.js';

export async function loadDatasetUrl(url: string): Promise<Dataset> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`dataset fetch ${res.status}`);
  let buf = new Uint8Array(await res.arrayBuffer());
  if (buf[0] === 0x1f && buf[1] === 0x8b) {
    const s = new Blob([buf]).stream().pipeThrough(new DecompressionStream('gzip'));
    buf = new Uint8Array(await new Response(s).arrayBuffer());
  }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', buf));
  const sha = Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('');
  return Dataset.fromJson(JSON.parse(new TextDecoder().decode(buf)), sha);
}
