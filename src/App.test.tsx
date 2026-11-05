import { render, screen } from "@testing-library/react";
import App from "./App";

describe("App", () => {
  it("renders the Memdock title", () => {
    render(<App />);
    expect(screen.getByText("Memdock")).toBeInTheDocument();
  });
});
