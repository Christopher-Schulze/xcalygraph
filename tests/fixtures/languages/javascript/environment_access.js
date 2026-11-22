const saved = process.env.NODE_ENV;
process.env.NODE_ENV = 'development';
process.env['NODE_ENV'] = 'production';
const bracket = process.env['NODE_ENV'];
process.env.NODE_ENV += '-next';
const compoundRight = process.env.NODE_ENV;
process.env.NODE_ENV++;
