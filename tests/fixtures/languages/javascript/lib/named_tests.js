it('not a test-role file', function () {
  process.env.NAMED_TEST_SETTING = 'ordinary';
});
it.each(['ordinary'])('also not a test-role file', () => {
  return process.env.NAMED_TEST_SETTING;
});
