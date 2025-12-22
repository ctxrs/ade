import { StyleSheet } from "react-native";
import { tokens, type ContextTokens } from "./tokens";

export { tokens };
export type { ContextTokens };

export const useContextTokens = (): ContextTokens => tokens;

export type ContextStyleSheet = Record<string, any>;

export function createContextStyles(factory: (theme: ContextTokens) => ContextStyleSheet): ContextStyleSheet {
  return StyleSheet.create(factory(tokens)) as ContextStyleSheet;
}
