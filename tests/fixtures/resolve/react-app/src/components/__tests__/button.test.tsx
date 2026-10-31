import { render, screen } from "@testing-library/react"

import { Button } from "../button"

test("renders the label", () => {
  render(<Button label="Save" />)
  expect(screen.getByText("Save")).toBeTruthy()
})
