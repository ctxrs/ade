declare module "@ctx/design/native" {
  import { StyleSheet } from "react-native";
  import type { ContextTokens } from "@ctx/design";

  export { tokens } from "@ctx/design";
  export type { ContextTokens };

  export function useContextTokens(): ContextTokens;
  export function createContextStyles<T extends Record<string, any>>(
    factory: (theme: ContextTokens) => T,
  ): T;
}
