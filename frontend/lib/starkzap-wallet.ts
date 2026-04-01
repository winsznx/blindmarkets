'use client';

import { RpcProvider, WalletAccount, typedData } from 'starknet';
import type { Call } from 'starknet';
import type { IntentPayload } from '@/lib/intentCrypto';

export type WalletProviderKey = 'email' | 'argent' | 'braavos';

export type ConnectedWallet = {
  address: string;
  execute: (calls: Call[], options?: { feeMode?: string }) => Promise<{ hash: string; wait: () => Promise<unknown> }>;
  signMessage?: (td: Parameters<typeof typedData.getMessageHash>[0]) => Promise<unknown>;
};

// ---- Injected wallet types ----

type InjectedStarknetWallet = {
  enable: (options?: { starknetVersion?: string }) => Promise<string[]>;
  selectedAddress?: string;
  [key: string]: unknown;
};

type InjectedWindow = Window & {
  starknet_argentX?: InjectedStarknetWallet;
  starknet_braavos?: InjectedStarknetWallet;
};

const STARKNET_RPC =
  process.env.NEXT_PUBLIC_STARKNET_RPC ??
  'https://starknet-sepolia.public.blastapi.io/rpc/v0_7';

function buildInjectedWallet(
  walletAccount: WalletAccount,
  rpcProvider: RpcProvider,
): ConnectedWallet {
  return {
    address: walletAccount.address,
    execute: async (calls: Call[], _options?: { feeMode?: string }) => {
      const result = await walletAccount.execute(calls);
      const hash = result.transaction_hash;
      return {
        hash,
        wait: () => rpcProvider.waitForTransaction(hash),
      };
    },
    signMessage: (td) => walletAccount.signMessage(td),
  };
}

// ---- Connect functions ----

export async function connectWithEmail(): Promise<ConnectedWallet> {
  throw new Error(
    'Email login requires Privy configuration. Set NEXT_PUBLIC_PRIVY_APP_ID in .env.local.',
  );
}

export async function connectWithArgent(): Promise<ConnectedWallet> {
  if (typeof window === 'undefined') throw new Error('Wallet connection requires a browser environment');
  const starknetWindow = (window as InjectedWindow).starknet_argentX;
  if (!starknetWindow) throw new Error('Argent X not installed');

  const accounts = await starknetWindow.enable();

  const rpcProvider = new RpcProvider({ nodeUrl: STARKNET_RPC });
  const walletAccount = new WalletAccount(
    rpcProvider,
    starknetWindow as unknown as ConstructorParameters<typeof WalletAccount>[1],
  );

  const built = buildInjectedWallet(walletAccount, rpcProvider);
  // WalletAccount reads selectedAddress from the injected window. Some wallet extensions
  // (e.g. Ready Wallet / Argent) set selectedAddress asynchronously after enable() resolves.
  // Fall back to the address returned by enable() if the WalletAccount didn't pick it up.
  const address = built.address || starknetWindow.selectedAddress || accounts[0] || '';
  if (!address) throw new Error('No account returned by Argent X');
  return { ...built, address };
}

export async function connectWithBraavos(): Promise<ConnectedWallet> {
  if (typeof window === 'undefined') throw new Error('Wallet connection requires a browser environment');
  const starknetWindow = (window as InjectedWindow).starknet_braavos;
  if (!starknetWindow) throw new Error('Braavos not installed');

  const accounts = await starknetWindow.enable();

  const rpcProvider = new RpcProvider({ nodeUrl: STARKNET_RPC });
  const walletAccount = new WalletAccount(
    rpcProvider,
    starknetWindow as unknown as ConstructorParameters<typeof WalletAccount>[1],
  );

  const built = buildInjectedWallet(walletAccount, rpcProvider);
  const address = built.address || starknetWindow.selectedAddress || accounts[0] || '';
  if (!address) throw new Error('No account returned by Braavos');
  return { ...built, address };
}

// ---- Session stub — Starkzap manages session internally ----

export async function restoreWalletSession(_providerKey: WalletProviderKey) {
  return null;
}

// ---- Utilities used by IntentComposer ----

export async function signIntentAuthorization(
  wallet: { signMessage(msg: Parameters<typeof typedData.getMessageHash>[0]): Promise<unknown> },
  intent: IntentPayload,
): Promise<{ signature: string[]; authorizationHash: string }> {
  const authorization = buildIntentAuthorizationTypedData(intent);
  const signature = await wallet.signMessage(authorization);
  const hash = typedData.getMessageHash(authorization, intent.userAddress);
  return {
    signature: normalizeSignature(signature),
    authorizationHash: normalizeHex(String(hash)),
  };
}

export function buildCancelIntentCall(intentId: string) {
  return {
    contractAddress: resolveIntentRegistryAddress(),
    entrypoint: 'cancel_intent',
    calldata: [normalizeHex(intentId), '0x0'],
  };
}

export function truncateAddress(address: string): string {
  if (!address || address.length < 12) return address;
  return `${address.slice(0, 6)}...${address.slice(-4)}`;
}

// ---- Private helpers ----

function normalizeSignature(signature: unknown): string[] {
  if (Array.isArray(signature)) return signature.map((v) => normalizeHex(String(v)));
  return [normalizeHex(String(signature))];
}

function buildIntentAuthorizationTypedData(intent: IntentPayload) {
  return {
    types: {
      StarkNetDomain: [
        { name: 'name', type: 'string' },
        { name: 'chainId', type: 'felt' },
        { name: 'version', type: 'string' },
      ],
      IntentAuthorization: [
        { name: 'intent_hash', type: 'felt' },
        { name: 'intent_id', type: 'felt' },
        { name: 'nonce', type: 'felt' },
        { name: 'amount_commitment', type: 'felt' },
        { name: 'deadline', type: 'felt' },
      ],
    },
    primaryType: 'IntentAuthorization',
    domain: {
      name: 'BlindMarkets',
      chainId: resolveChainId(),
      version: '1',
    },
    message: {
      intent_hash: intent.intentHash,
      intent_id: intent.intentId,
      nonce: intent.nonce,
      amount_commitment: intent.amountCommitment,
      deadline: bigintToHex(intent.deadline),
    },
  };
}

function resolveIntentRegistryAddress(): string {
  const addr =
    process.env.NEXT_PUBLIC_INTENT_REGISTRY ?? process.env.NEXT_PUBLIC_INTENT_REGISTRY_ADDRESS;
  if (!addr) throw new Error('NEXT_PUBLIC_INTENT_REGISTRY is not configured');
  return normalizeHex(addr);
}

function resolveChainId(): string {
  const chainId = process.env.NEXT_PUBLIC_STARKNET_CHAIN_ID;
  if (!chainId) throw new Error('NEXT_PUBLIC_STARKNET_CHAIN_ID is not configured');
  return normalizeHex(chainId);
}

function normalizeHex(value: string): string {
  const s = value.trim();
  if (!s) return s;
  if (s.startsWith('0x') || s.startsWith('0X')) return `0x${s.slice(2).toLowerCase()}`;
  if (/^\d+$/.test(s)) return `0x${BigInt(s).toString(16)}`;
  return `0x${s.toLowerCase()}`;
}

function bigintToHex(value: bigint): string {
  return normalizeHex(value.toString(16));
}
