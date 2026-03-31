/** @type {import('next').NextConfig} */
const path = require('path');

const nextConfig = {
  reactStrictMode: true,
  swcMinify: true,
  output: 'standalone',
  async redirects() {
    return [
      {
        source: '/favicon.ico',
        destination: '/icon.svg',
        permanent: false,
      },
    ];
  },
  webpack: (config) => {
    // starknet v6 ships a "browser" export condition pointing to index.global.js,
    // an IIFE with no module exports. Pin our app code to the CJS build directly.
    //
    // starkzap has its own nested starknet v9 at node_modules/starkzap/node_modules/starknet.
    // v9 has NO "browser" export condition, so webpack resolves it to the real ESM/CJS build
    // naturally. The alias is scoped to the top-level package name and does not override
    // nested resolution inside starkzap's own node_modules subtree.
    //
    // NOTE: transpilePackages: ['starknet'] was removed. Next.js's transpilePackages
    // implementation adds an internal alias that overrides nested resolution and forces
    // starkzap back to v6's IIFE, causing AbiParser2 not a constructor at runtime.
    config.resolve.alias['starknet'] = path.resolve(
      __dirname,
      'node_modules/starknet/dist/index.js'
    );

    // Stub optional GCP logging dep pulled in by @hyperlane-xyz/utils (starkzap Solana bridge).
    config.resolve.alias['@google-cloud/pino-logging-gcp-config'] = false;

    return config;
  },
};

module.exports = nextConfig;
