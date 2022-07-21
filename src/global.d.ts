export interface Config {
  programID: string;
  assets: Asset[];
  oracles: Oracles;
  markets: Market[];
}
export interface Asset {
  name: string;
  symbol: string;
  decimals: number;
  mintAddress: string;
}
export interface Oracles {
  pythProgramID: string;
  switchboardProgramID: string;
  assets: OracleAsset[];
}
export interface OracleAsset {
  asset: string;
  priceAddress: string;
  switchboardFeedAddress: string;
}
export interface Market {
  name: string;
  address: string;
  authorityAddress: string;
  reserves: Reserve[];
}
export interface Reserve {
  asset: string;
  address: string;
  collateralMintAddress: string;
  collateralSupplyAddress: string;
  liquidityAddress: string;
  liquidityFeeReceiverAddress: string;
  userSupplyCap?: number;
}
