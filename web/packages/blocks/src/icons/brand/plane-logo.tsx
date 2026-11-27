/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

import * as React from "react";
import { SATURN_MARK_DATA_URI } from "./saturn-mark-data";

export type PlaneLogoProps = React.ComponentPropsWithoutRef<"svg">;

export function PlaneLogo({ width = "52", height = "52", className, ...props }: PlaneLogoProps) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="0 0 52 52"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      {...props}
    >
      <image href={SATURN_MARK_DATA_URI} x="0" y="0" width="52" height="52" />
    </svg>
  );
}
