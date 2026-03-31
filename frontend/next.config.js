/** @type {import('next').NextConfig} */
const path = require('path');
const webpack = require('webpack');

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
    // an IIFE with no module exports, making WalletAccount etc. undefined at runtime.
    // Pin our app code to the CJS build directly.
    //
    // starkzap has a nested starknet v9. Both v6 and v9 have `require("fs")` inside
    // file-reading helper functions. These are never called in a browser wallet flow,
    // but webpack still tries to resolve `fs` at bundle time. Stub it out.
    config.resolve.alias['starknet'] = path.resolve(
      __dirname,
      'node_modules/starknet/dist/index.js'
    );

    // Stub Node built-ins that appear in starknet v9 file-reading helpers.
    // Those code paths are unreachable in browser usage.
    config.resolve.fallback = {
      ...config.resolve.fallback,
      fs: false,
      path: false,
    };

    // Stub optional GCP logging dep pulled in by @hyperlane-xyz/utils (starkzap Solana bridge).
    config.resolve.alias['@google-cloud/pino-logging-gcp-config'] = false;

    return config;
  },
};

module.exports = nextConfig;
