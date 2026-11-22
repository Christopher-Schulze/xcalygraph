it('named it callback', function () {
  process.env.NAMED_TEST_SETTING = 'development';
});
test('named test callback', () => {
  return process.env.NAMED_TEST_SETTING;
});
it.each(['development'])('named %s callback', function (mode) {
  process.env.NAMED_TEST_SETTING = mode;
});
it('escaped \u0063allback', function () {
  process.env.NAMED_TEST_SETTING = 'escaped';
});
describe('not a test symbol', function () {
  return process.env.NAMED_TEST_SETTING;
});
before(function () {
  process.env.NAMED_TEST_SETTING = 'setup';
});
after(() => {
  process.env.NAMED_TEST_SETTING = 'cleanup';
});
service.handle('not a test callback', function () {
  process.env.NAMED_TEST_SETTING = 'ordinary';
});
it(dynamicTitle, function () {
  process.env.NAMED_TEST_SETTING = 'dynamic';
});
it.each(['x'], function () {
  process.env.NAMED_TEST_SETTING = 'inner';
})('wrong callback position', 'not a function');
