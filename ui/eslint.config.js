import js from "@eslint/js";
import tseslint from "@typescript-eslint/eslint-plugin";
import tsParser from "@typescript-eslint/parser";
import vue from "eslint-plugin-vue";
import vueParser from "vue-eslint-parser";

export default [
  {
    ignores: ["dist/**", "node_modules/**", "src/client/**", "openapi.json"],
  },
  js.configs.recommended,
  ...vue.configs["flat/recommended"],
  {
    files: ["src/**/*.{ts,vue}"],
    languageOptions: {
      parser: vueParser,
      parserOptions: {
        ecmaVersion: "latest",
        extraFileExtensions: [".vue"],
        parser: tsParser,
        sourceType: "module",
      },
    },
    plugins: {
      "@typescript-eslint": tseslint,
    },
    rules: {
      ...tseslint.configs.recommended.rules,
      "@typescript-eslint/no-unused-vars": [
        "error",
        { varsIgnorePattern: "^_", argsIgnorePattern: "^_" },
      ],
      // TypeScript (and `vue-tsc`) already catch undefined variables/types —
      // `no-undef` can't see ambient DOM types like `RequestInit` and produces
      // false positives. See typescript-eslint's FAQ on this rule.
      "no-undef": "off",
      // Formatting is owned by `oxfmt`, not ESLint — these stylistic rules
      // from `flat/recommended` disagree with its output and would fire on
      // nearly every template, burying real findings in noise.
      "vue/max-attributes-per-line": "off",
      "vue/singleline-html-element-content-newline": "off",
      "vue/multiline-html-element-content-newline": "off",
      "vue/html-closing-bracket-newline": "off",
      "vue/html-indent": "off",
      "vue/html-self-closing": "off",
    },
  },
  {
    // shadcn/ui auto-generated components: relax rules that conflict with the
    // generated patterns (single-word names, optional class passthroughs,
    // intentional v-html in rendering primitives like CodeBlock).
    files: ["src/components/ui/**/*.vue"],
    rules: {
      "vue/multi-word-component-names": "off",
      "vue/require-default-prop": "off",
      "vue/no-v-html": "off",
    },
  },
];
