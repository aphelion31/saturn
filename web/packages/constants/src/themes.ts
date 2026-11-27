/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

export const THEMES = ["light", "dark", "light-contrast", "dark-contrast", "custom"];

export interface I_THEME_OPTION {
  key: string;
  value: string;
  i18n_label: string;
  type: string;
  icon: {
    border: string;
    color1: string;
    color2: string;
  };
}

export const THEME_OPTIONS: I_THEME_OPTION[] = [
  {
    key: "system_preference",
    value: "system",
    i18n_label: "System preference",
    type: "light",
    icon: {
      border: "#DDD2C3",
      color1: "#F5EFE4",
      color2: "#C96E32",
    },
  },
  {
    key: "light",
    value: "light",
    i18n_label: "Light",
    type: "light",
    icon: {
      border: "#DDD2C3",
      color1: "#F5EFE4",
      color2: "#C96E32",
    },
  },
  {
    key: "dark",
    value: "dark",
    i18n_label: "Dark",
    type: "dark",
    icon: {
      border: "#383029",
      color1: "#14110F",
      color2: "#D47E43",
    },
  },
  {
    key: "light_contrast",
    value: "light-contrast",
    i18n_label: "Light high contrast",
    type: "light",
    icon: {
      border: "#0A0A0A",
      color1: "#FFFEFB",
      color2: "#AC531B",
    },
  },
  {
    key: "dark_contrast",
    value: "dark-contrast",
    i18n_label: "Dark high contrast",
    type: "dark",
    icon: {
      border: "#F5EFE4",
      color1: "#0C0A08",
      color2: "#E5955A",
    },
  },
  {
    key: "custom",
    value: "custom",
    i18n_label: "Custom theme",
    type: "light",
    icon: {
      border: "#FFC9C9",
      color1: "#FFF7F7",
      color2: "#FF5151",
    },
  },
];
