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
    // A global resolve.alias['starknet'] would also redirect starkzap's own starknet
    // imports — starkzap ships a nested v9 that uses APIs (AbiParser2, etc.) absent
    // from v6, causing "n.AbiParser2 is not a constructor" at runtime.
    // Use NormalModuleReplacementPlugin instead, scoped to non-starkzap callers.
    config.plugins.push(
      new webpack.NormalModuleReplacementPlugin(
        /^starknet$/,
        (resource) => {
          if (
            resource.context &&
            resource.context.includes(path.join('node_modules', 'starkzap'))
          ) {
            return;
          }
          resource.request = path.resolve(
            __dirname,
            'node_modules/starknet/dist/index.js'
          );
        }
      )
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
