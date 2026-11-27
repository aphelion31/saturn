/**
 * Copyright (c) 2023-present Plane Software, Inc. and contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 * See the LICENSE file for details.
 */

import * as React from "react";
import { SATURN_MARK_DATA_URI } from "./saturn-mark-data";

export type PlaneLockupProps = React.ComponentPropsWithoutRef<"svg">;

export function PlaneLockup({
  width = "185",
  height = "52",
  className,
  color = "currentColor",
  ...props
}: PlaneLockupProps) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="0 0 185 52"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      preserveAspectRatio="xMinYMid meet"
      {...props}
    >
      <image href={SATURN_MARK_DATA_URI} x="0" y="0" width="52" height="52" />
      <text
        x="64"
        y="38.5"
        fill={color}
        fontFamily="Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
        fontSize="35"
        fontWeight="700"
        letterSpacing="-0.025em"
      >
        Saturn
      </text>
    </svg>
  );
}
