/** @type {import('next').NextConfig} */
const path = require('path');

const nextConfig = {
  reactStrictMode: true,
  swcMinify: true,
  output: 'standalone',
  transpilePackages: ['starknet'],
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
    config.resolve.alias['starknet'] = path.resolve(
      __dirname,
      'node_modules/starknet/dist/index.js'
    );
    // Stub optional GCP logging dep pulled in by @hyperlane-xyz/utils (starkzap Solana bridge).
    // This package is not installed and is not needed for Starknet-only usage.
    config.resolve.alias['@google-cloud/pino-logging-gcp-config'] = false;
    return config;
  },
};

module.exports = nextConfig;
