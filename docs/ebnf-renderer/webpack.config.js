export default {
  mode: "production",
  resolve: {
    fallback: {
      "fs": false,
      "path": false,
    }
  },
};