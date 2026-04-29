'use strict';

const addon = require('bindings')('prosperous');

class ProsperousClient {
  constructor(options = {}) {
    this._options = {
      prosperousKey: options.prosperousKey ?? null,
      baseUrl: options.baseUrl ?? null,
    };
    this._state = null;
  }

  async initialize() {
    const json = await addon.initialize(this._options);
    const result = JSON.parse(json);

    if (result.ok) {
      this._state = {
        type: result.state,
        email: result.email ?? null,
        orgId: result.orgId ?? null,
        exp: result.exp ?? null,
      };
      return this._state;
    }

    const err = new Error(result.error);
    err.code = result.error;
    if (result.email) err.email = result.email;
    if (result.orgId) err.orgId = result.orgId;
    throw err;
  }

  get state() {
    return this._state;
  }
}

module.exports = { ProsperousClient };
