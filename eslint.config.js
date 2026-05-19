import js from "@eslint/js";
import tseslint from "typescript-eslint";
import solid from "eslint-plugin-solid/configs/typescript";
import prettier from "eslint-config-prettier";

export default tseslint.config(
  { ignores: ["dist/**", "node_modules/**", "src-tauri/**", "coverage/**", ".worktrees/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    ...solid,
    languageOptions: {
      parserOptions: {
        project: false,
        ecmaVersion: 2022,
        sourceType: "module",
      },
      globals: {
        window: "readonly",
        document: "readonly",
        localStorage: "readonly",
        console: "readonly",
        setTimeout: "readonly",
        clearTimeout: "readonly",
        setInterval: "readonly",
        clearInterval: "readonly",
        fetch: "readonly",
        navigator: "readonly",
        location: "readonly",
        URL: "readonly",
        URLSearchParams: "readonly",
        HTMLElement: "readonly",
        Element: "readonly",
        Node: "readonly",
        Event: "readonly",
        CustomEvent: "readonly",
      },
    },
    rules: {
      // App is small; we'd rather catch unused vars but not block on naming style.
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // The Solid plugin's reactivity rule produces many false positives in this
      // codebase (stores accessed inside event handlers). Demote to warn.
      "solid/reactivity": "warn",
      // Same — Tauri command shims return `unknown` until we type them; allow.
      "@typescript-eslint/no-explicit-any": "warn",
    },
  },
  {
    files: ["src/test/**/*.ts", "src/**/*.test.{ts,tsx}"],
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-empty-function": "off",
    },
  },
  prettier,
);
