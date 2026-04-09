/* eslint-disable no-console */
import * as esbuild from 'esbuild';
import { statSync } from 'node:fs';

console.log('Building Privy SDK bundle…');

await esbuild.build({
  entryPoints:  ['entry.mjs'],
  bundle:       true,
  format:       'esm',
  outfile:      '../static/privy-login.js',
  minify:       true,
  sourcemap:    false,
  define: {
    'process.env.NODE_ENV': '"production"',
    'global':               'globalThis',
    'process.browser':      'true',
  },
  target:   ['es2020', 'chrome90', 'firefox90', 'safari14'],
  platform: 'browser',
  // Tree-shake aggressively; don't include unused login methods
  treeShaking: true,
});

const { size } = statSync('../static/privy-login.js');
console.log(`✓  static/privy-login.js  (${(size / 1024).toFixed(0)} KB)`);
console.log('   Commit this file: git add static/privy-login.js');
