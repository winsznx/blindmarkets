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
  webpack: (config, { webpack }) => {
    // starknet v6 ships a "browser" export condition pointing to index.global.js,
    // an IIFE with no module exports, making WalletAccount etc. undefined at runtime.
    // Using resolve.alias here would override starkzap's own nested starknet v9
    // (node_modules/starkzap/node_modules/starknet), causing AbiParser2 not a
    // constructor at runtime. Use NormalModuleReplacementPlugin instead so we
    // can scope the redirect to importers outside starkzap's own package tree.
    config.plugins.push(
      new webpack.NormalModuleReplacementPlugin(/^starknet$/, (resource) => {
        const issuer = resource.context ?? '';
        if (!issuer.includes('/node_modules/starkzap')) {
          resource.request = path.resolve(
            __dirname,
            'node_modules/starknet/dist/index.js'
          );
        }
      })
    );

    // Stub optional GCP logging dep pulled in by @hyperlane-xyz/utils (starkzap Solana bridge).
    // This package is not installed and is not needed for Starknet-only usage.
    config.resolve.alias['@google-cloud/pino-logging-gcp-config'] = false;
    return config;
  },
};

module.exports = nextConfig;
