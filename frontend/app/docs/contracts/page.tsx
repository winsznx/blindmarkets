const addr = (address: string) => (
  <a
    href={`https://sepolia.voyager.online/contract/${address}`}
    target="_blank"
    rel="noreferrer"
    className="font-mono text-xs break-all"
    style={{ color: 'var(--accent-primary)' }}
  >
    {address}
  </a>
);

export default function ContractsPage() {
  return (
    <article className="prose-docs">
      <h1>Deployed Addresses</h1>
      <p className="lead">
        All contracts are live on <strong>Starknet Sepolia</strong> testnet. Click any address to
        view it on Voyager.
      </p>

      <hr />

      <div className="not-prose space-y-3 my-6">
        {[
          {
            name: 'IntentRegistry',
            address: '0x02ba178fd7b7a44483747d1bba06ec08a61917ebf3e6489fbf118a25f2613f2b',
            desc: 'Your wallet talks to this contract when you place an order. It stores the on-chain commitment hash.',
          },
          {
            name: 'BatchAuction',
            address: '0x07e7783df0c502dc41e13fb37a5503e4e3d3d48994589d5ffddcab1c67be339c',
            desc: 'Runs the solver auction. Receives solutions, scores them, declares a winner per batch.',
          },
          {
            name: 'BatchSettlement',
            address: '0x05d779212e1adf78803bb6051550177c0a36cbb7dfd920ec2665ed76aa9e7eb5',
            desc: 'Executes the winning solution. Transfers tokens atomically, verifies fills against commitments.',
          },
          {
            name: 'SolverBond',
            address: '0x069db55d725a0a4f48705ebecf0993e455e2b3716940081005b48d779e7301e8',
            desc: 'Manages solver collateral. Solvers deposit here; bad behaviour is slashed from this balance.',
          },
          {
            name: 'AssetRegistry',
            address: '0x061b3c120cd166e1c523509a1ee153060b8afeab4f534c28d55d24de61bd4246',
            desc: 'Whitelist of tradeable tokens. A token must be registered here before it can appear in any order.',
          },
          {
            name: 'GatewayRegistry',
            address: '0x0572569d692b6711f7da40d9d196bccf56b703cf8dafe03580aa713e974557b0',
            desc: 'Registry of authorised gateway operators. A gateway must be registered here before it can relay intents.',
          },
        ].map((c) => (
          <div key={c.name} className="glass-card rounded-2xl px-5 py-4">
            <div className="flex items-start justify-between gap-4 mb-2">
              <span className="font-semibold text-text-primary text-sm">{c.name}</span>
            </div>
            <p className="text-text-muted text-sm mb-2">{c.desc}</p>
            {addr(c.address)}
          </div>
        ))}
      </div>

      <hr />

      <h2>Network</h2>
      <table>
        <tbody>
          <tr>
            <td>Network</td>
            <td><strong>Starknet Sepolia testnet</strong></td>
          </tr>
          <tr>
            <td>Chain ID</td>
            <td><code>0x534e5f5345504f4c4941</code></td>
          </tr>
          <tr>
            <td>RPC</td>
            <td><code>https://starknet-sepolia.drpc.org</code></td>
          </tr>
          <tr>
            <td>Block explorer</td>
            <td>
              <a href="https://sepolia.voyager.online" target="_blank" rel="noreferrer">
                sepolia.voyager.online
              </a>
            </td>
          </tr>
        </tbody>
      </table>

      <hr />

      <h2>Contract interactions</h2>
      <p>The contracts are wired together. Here's who calls what:</p>
      <table>
        <thead>
          <tr><th>Caller</th><th>Contract</th><th>When</th></tr>
        </thead>
        <tbody>
          <tr><td>User wallet</td><td>IntentRegistry</td><td>When placing an order</td></tr>
          <tr><td>Coordinator</td><td>IntentRegistry</td><td>To open and close batch windows</td></tr>
          <tr><td>Solver</td><td>BatchAuction</td><td>To submit a solution</td></tr>
          <tr><td>BatchAuction</td><td>BatchSettlement</td><td>When declaring an auction winner</td></tr>
          <tr><td>BatchSettlement</td><td>SolverBond</td><td>To slash a solver that fails to settle</td></tr>
          <tr><td>Solver</td><td>SolverBond</td><td>To deposit or withdraw their bond</td></tr>
        </tbody>
      </table>

      <hr />

      <h2>Supported tokens</h2>
      <p>
        Every token used in an intent must be whitelisted in the <strong>AssetRegistry</strong>{' '}
        contract. <code>IntentRegistry.commit_intent</code> calls{' '}
        <code>AssetRegistry.is_whitelisted</code> for both <code>asset_in</code> and{' '}
        <code>asset_out</code> before accepting the intent — an unlisted token is rejected
        on-chain.
      </p>

      <table>
        <thead>
          <tr><th>Token</th><th>Symbol</th><th>Address (Sepolia)</th><th>Decimals</th><th>Status</th></tr>
        </thead>
        <tbody>
          <tr>
            <td>USD Coin</td>
            <td><code>USDC</code></td>
            <td><code className="text-xs">0x053b40a647cedfca6ca84f542a0fe36736031905a9639a7f19a3c1e66bfd5080</code></td>
            <td>6</td>
            <td>Pending whitelist</td>
          </tr>
          <tr>
            <td>Wrapped BTC</td>
            <td><code>WBTC</code></td>
            <td>Bridge from Ethereum Sepolia via <a href="https://sepolia.starkgate.starknet.io" target="_blank" rel="noreferrer">Starkgate</a></td>
            <td>8</td>
            <td>Pending whitelist</td>
          </tr>
        </tbody>
      </table>

      <p>
        To whitelist a token, the admin account calls <code>whitelist_asset</code> on the
        AssetRegistry. Once whitelisted, solvers can fill intents for that pair and settlement will
        execute the token transfer.
      </p>

      <hr />

      <h2>Source code</h2>
      <p>
        All contracts are written in Cairo and live in the <code>contracts/src/</code> directory
        of the monorepo. They're open source under the MIT licence.
      </p>
    </article>
  );
}
