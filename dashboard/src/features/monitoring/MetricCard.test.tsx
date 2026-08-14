import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { MetricCard } from "./MetricCard";

describe("MetricCard", () => {
  it("renders label, value and sub", () => {
    render(<MetricCard label="Verified" value="213" sub="total 5000" />);
    expect(screen.getByText("Verified")).toBeInTheDocument();
    expect(screen.getByText("213")).toBeInTheDocument();
    expect(screen.getByText("total 5000")).toBeInTheDocument();
  });

  it("renders without sub", () => {
    render(<MetricCard label="RAM" value="201 MiB" />);
    expect(screen.getByText("RAM")).toBeInTheDocument();
    expect(screen.getByText("201 MiB")).toBeInTheDocument();
  });
});
