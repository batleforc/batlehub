const express = require("express");
const _ = require("lodash");

const app = express();
const port = process.env.PORT || 3000;

app.get("/", (_req, res) => {
  res.json({ message: "hello", version: _.VERSION });
});

app.listen(port, () => {
  console.log(`Listening on http://localhost:${port}`);
});
