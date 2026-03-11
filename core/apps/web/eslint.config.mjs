import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";
import noRawAnchorHrefRule from "./eslint/rules/no-raw-anchor-href.js";

const webFiles = ["src/**/*.{ts,tsx}"];

export default tseslint.config(
  {
    ignores: ["coverage/**", "dist/**", "node_modules/**", "public/**", ".turbo/**"],
    linterOptions: {
      reportUnusedDisableDirectives: "off",
    },
  },
  ...tseslint.configs.recommended,
  {
    files: webFiles,
    languageOptions: {
      globals: {
        ...globals.browser,
      },
      parserOptions: {
        ecmaFeatures: {
          jsx: true,
        },
        ecmaVersion: "latest",
        sourceType: "module",
      },
    },
    plugins: {
      "ctx-web": {
        rules: {
          "no-raw-anchor-href": noRawAnchorHrefRule,
        },
      },
      "react-hooks": reactHooks,
    },
    rules: {
      "@typescript-eslint/no-unused-vars": "off",
      "@typescript-eslint/prefer-as-const": "off",
      "no-unused-vars": "off",
      "prefer-const": "off",
      "constructor-super": "error",
      "ctx-web/no-raw-anchor-href": "error",
      "getter-return": "error",
      "no-async-promise-executor": "error",
      "no-constant-binary-expression": "error",
      "no-debugger": "error",
      "no-dupe-args": "error",
      "no-dupe-keys": "error",
      "no-unreachable": "error",
      "no-unsafe-finally": "error",
      "react-hooks/rules-of-hooks": "error",
      "use-isnan": "error",
      "valid-typeof": "error",
    },
  },
);
