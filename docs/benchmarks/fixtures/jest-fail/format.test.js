const { formatCurrency } = require("./format");
test("formats USD", () => {
  expect(formatCurrency(1234.56)).toBe("$1,234.56");
});
