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
    // starknet v6 exports a "browser" condition that points to `dist/index.global.js`,
    // an IIFE with no module exports. This breaks bundlers that prefer "browser" for
    // client builds.
    //
    // We must:
    // - Force *our* dependency graph to use the module build (`dist/index.js`)
    // - Avoid hijacking starkzap's own nested starknet v9 (AbiParser2 mismatch)
    //
    // Webpack aliases are global, so instead we use a scoped replacement: only rewrite
    // requests for `starknet` when the issuer is NOT inside `node_modules/starkzap`.
    config.plugins = config.plugins || [];
    config.plugins.push(
      new webpack.NormalModuleReplacementPlugin(/^starknet$/, (resource) => {
        const context = resource.context || '';
        if (context.includes(`${path.sep}node_modules${path.sep}starkzap`)) return;
        resource.request = path.resolve(
          __dirname,
          'node_modules/starknet/dist/index.js'
        );
      })
    );

    // Stub optional GCP logging dep pulled in by @hyperlane-xyz/utils (starkzap Solana bridge).
    config.resolve.alias['@google-cloud/pino-logging-gcp-config'] = false;

    return config;
  },
};

module.exports = nextConfig;
