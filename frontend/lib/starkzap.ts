import { StarkZap, OnboardStrategy } from 'starkzap';

export const starkzap = new StarkZap({
  network: (process.env.NEXT_PUBLIC_STARKNET_NETWORK ?? 'sepolia') as 'mainnet' | 'sepolia',
  ...(process.env.NEXT_PUBLIC_AVNU_PAYMASTER_URL && {
    paymaster: { nodeUrl: process.env.NEXT_PUBLIC_AVNU_PAYMASTER_URL },
  }),
});

export { OnboardStrategy };
export type StarkzapWallet = Awaited<ReturnType<typeof starkzap.onboard>>['wallet'];
