Ext.define('PBS.LoginView', {
    extend: 'Ext.container.Container',
    xtype: 'loginview',

    viewModel: {
	data: {
	    openid: false,
	},
	formulas: {
	    button_text: function(get) {
		if (get("openid") === true) {
		    return gettext("Login (OpenID redirect)");
		} else {
		    return gettext("Login");
		}
	    },
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	submitForm: async function() {
	    var me = this;
	    var loginForm = me.lookupReference('loginForm');
	    var unField = me.lookupReference('usernameField');
	    var saveunField = me.lookupReference('saveunField');

	    if (!loginForm.isValid()) {
		return;
	    }

	    let creds = loginForm.getValues();

	    if (this.getViewModel().data.openid === true) {
		const redirectURL = location.origin;
		try {
		    let resp = await PBS.Async.api2({
			url: '/api2/extjs/access/openid/auth-url',
			params: {
			    realm: creds.realm,
			    "redirect-url": redirectURL,
			},
			method: 'POST',
		    });
		    window.location = resp.result.data;
		} catch (error) {
		    Proxmox.Utils.authClear();
		    loginForm.unmask();
		    Ext.MessageBox.alert(
			gettext('Error'),
			gettext('OpenID redirect failed, please try again') + `<br>${error}`,
		    );
		}
		return;
	    }

	    creds.username = `${creds.username}@${creds.realm}`;
	    delete creds.realm;

	    if (loginForm.isVisible()) {
		loginForm.mask(gettext('Please wait...'), 'x-mask-loading');
	    }

	    // set or clear username
	    var sp = Ext.state.Manager.getProvider();
	    if (saveunField.getValue() === true) {
		sp.set(unField.getStateId(), unField.getValue());
	    } else {
		sp.clear(unField.getStateId());
	    }
	    sp.set(saveunField.getStateId(), saveunField.getValue());

	    try {
		let resp = await PBS.Async.api2({
		    url: '/api2/extjs/access/ticket',
		    params: creds,
		    method: 'POST',
		});

		let data = resp.result.data;
		if (data.ticket.startsWith("PBS:!tfa!")) {
		    data = await me.performTFAChallenge(data);
		}

		PBS.Utils.updateLoginData(data);
		PBS.app.changeView('mainview');
	    } catch (error) {
		Proxmox.Utils.authClear();
		loginForm.unmask();
		Ext.MessageBox.alert(
		    gettext('Error'),
		    gettext('Login failed. Please try again'),
		);
	    }
	},

	performTFAChallenge: async function(data) {
	    let me = this;

	    let userid = data.username;
	    let ticket = data.ticket;
	    let challenge = JSON.parse(decodeURIComponent(
	        ticket.split(':')[1].slice("!tfa!".length),
	    ));

	    let resp = await new Promise((resolve, reject) => {
		Ext.create('PBS.login.TfaWindow', {
		    userid,
		    ticket,
		    challenge,
		    onResolve: value => resolve(value),
		    onReject: reject,
		}).show();
	    });

	    return resp.result.data;
	},

	control: {
	    'field[name=username]': {
		specialkey: function(f, e) {
		    if (e.getKey() === e.ENTER) {
			var pf = this.lookupReference('passwordField');
			if (!pf.getValue()) {
			    pf.focus(false);
			}
		    }
		},
	    },
	    'field[name=lang]': {
		change: function(f, value) {
		    var dt = Ext.Date.add(new Date(), Ext.Date.YEAR, 10);
		    Ext.util.Cookies.set('PBSLangCookie', value, dt);
		    this.getView().mask(gettext('Please wait...'), 'x-mask-loading');
		    window.location.reload();
		},
	    },
	    'field[name=realm]': {
		change: function(f, value) {
		    let record = f.store.getById(value);
		    if (record === undefined) return;
		    let data = record.data;
		    this.getViewModel().set("openid", data.type === "openid");
		},
	    },
	    'button[reference=loginButton]': {
		click: 'submitForm',
	    },
	    'window[reference=loginwindow]': {
		show: function() {
		    var sp = Ext.state.Manager.getProvider();
		    var checkboxField = this.lookupReference('saveunField');
		    var unField = this.lookupReference('usernameField');

		    var checked = sp.get(checkboxField.getStateId());
		    checkboxField.setValue(checked);

		    if (checked === true) {
			var username = sp.get(unField.getStateId());
			unField.setValue(username);
			var pwField = this.lookupReference('passwordField');
			pwField.focus();
		    }

		    let auth = Proxmox.Utils.getOpenIDRedirectionAuthorization();
		    if (auth !== undefined) {
			Proxmox.Utils.authClear();

			let loginForm = this.lookupReference('loginForm');
			loginForm.mask(gettext('OpenID login - please wait...'), 'x-mask-loading');

			// openID checks the original redirection URL we used, so pass that too
			const redirectURL = location.origin;

			Proxmox.Utils.API2Request({
			    url: '/api2/extjs/access/openid/login',
			    params: {
				state: auth.state,
				code: auth.code,
				"redirect-url": redirectURL,
			    },
			    method: 'POST',
			    failure: function(response) {
				loginForm.unmask();
				let error = response.htmlStatus;
				Ext.MessageBox.alert(
				    gettext('Error'),
				    gettext('OpenID login failed, please try again') + `<br>${error}`,
				    () => { window.location = redirectURL; },
				);
			    },
			    success: function(response, options) {
				loginForm.unmask();
				let creds = response.result.data;
				PBS.Utils.updateLoginData(creds);
				PBS.app.changeView('mainview');
				history.replaceState(null, '', `${redirectURL}#pbsDashboard`);
			    },
			});
		    }
		},
	    },
	},
    },

    plugins: 'viewport',

    layout: {
	type: 'border',
    },

    items: [
	{
	    region: 'north',
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'middle',
	    },
	    margin: '2 5 2 5',
	    height: 38,
	    items: [
		{
		    xtype: 'proxmoxlogo',
		    prefix: '',
		},
		{
		    xtype: 'versioninfo',
		    makeApiCall: false,
		},
	    ],
	},
	{
	    region: 'center',
	},
	{
	    xtype: 'window',
	    closable: false,
	    resizable: false,
	    reference: 'loginwindow',
	    autoShow: true,
	    modal: true,
	    width: 400,

	    defaultFocus: 'usernameField',

	    layout: {
		type: 'auto',
	    },

	    title: gettext('Proxmox Backup Server Login'),

	    items: [
		{
		    xtype: 'form',
		    layout: {
			type: 'form',
		    },
		    defaultButton: 'loginButton',
		    url: '/api2/extjs/access/ticket',
		    reference: 'loginForm',

		    fieldDefaults: {
			labelAlign: 'right',
			allowBlank: false,
		    },

		    items: [
			{
			    xtype: 'textfield',
			    fieldLabel: gettext('User name'),
			    name: 'username',
			    itemId: 'usernameField',
			    reference: 'usernameField',
			    stateId: 'login-username',
			    bind: {
				visible: "{!openid}",
				disabled: "{openid}",
			    },
			},
			{
			    xtype: 'textfield',
			    inputType: 'password',
			    fieldLabel: gettext('Password'),
			    name: 'password',
			    itemId: 'passwordField',
			    reference: 'passwordField',
			    bind: {
				visible: "{!openid}",
				disabled: "{openid}",
			    },
			},
			{
			    xtype: 'pmxRealmComboBox',
			    name: 'realm',
			},
			{
			    xtype: 'proxmoxLanguageSelector',
			    fieldLabel: gettext('Language'),
			    value: Ext.util.Cookies.get('PBSLangCookie') || Proxmox.defaultLang || 'en',
			    name: 'lang',
			    reference: 'langField',
			    submitValue: false,
			},
		    ],
		    buttons: [
			{
			    xtype: 'checkbox',
			    fieldLabel: gettext('Save User name'),
			    name: 'saveusername',
			    reference: 'saveunField',
			    stateId: 'login-saveusername',
			    labelWidth: 250,
			    labelAlign: 'right',
			    submitValue: false,
			    bind: {
				visible: "{!openid}",
			    },
			},
			{
			    bind: {
				text: "{button_text}",
			    },
			    reference: 'loginButton',
			    formBind: true,
			},
		    ],
		},
	    ],
	},
    ],
});

Ext.define('PBS.login.TfaWindow', {
    extend: 'Ext.window.Window',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext("Second login factor required"),

    modal: true,
    resizable: false,
    width: 512,
    layout: {
	type: 'vbox',
	align: 'stretch',
    },

    defaultButton: 'tfaButton',

    viewModel: {
	data: {
	    confirmText: gettext('Confirm Second Factor'),
	    canConfirm: false,
	    availableChallenge: {},
	},
    },

    cancelled: true,

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    let me = this;
	    let vm = me.getViewModel();

	    if (!view.userid) {
		throw "no userid given";
	    }
	    if (!view.ticket) {
		throw "no ticket given";
	    }
	    const challenge = view.challenge;
	    if (!challenge) {
		throw "no challenge given";
	    }

	    let lastTabId = me.getLastTabUsed();
	    let initialTab = -1, i = 0;
	    for (const k of ['webauthn', 'totp', 'recovery']) {
		const available = !!challenge[k];
		vm.set(`availableChallenge.${k}`, available);

		if (available) {
		    if (i === lastTabId) {
			initialTab = i;
		    } else if (initialTab < 0) {
			initialTab = i;
		    }
		}
		i++;
	    }
	    view.down('tabpanel').setActiveTab(initialTab);

	    if (challenge.recovery) {
		me.lookup('availableRecovery').update(Ext.String.htmlEncode(
		    gettext('Available recovery keys: ') + view.challenge.recovery.join(', '),
		));
		me.lookup('availableRecovery').setVisible(true);
		if (view.challenge.recovery.length <= 3) {
		    me.lookup('recoveryLow').setVisible(true);
		}
	    }

	    if (challenge.webauthn && initialTab === 0) {
		let _promise = me.loginWebauthn();
	    }
	},
	control: {
	    'tabpanel': {
		tabchange: function(tabPanel, newCard, oldCard) {
		    // for now every TFA method has at max one field, so keep it simple..
		    let oldField = oldCard.down('field');
		    if (oldField) {
			oldField.setDisabled(true);
		    }
		    let newField = newCard.down('field');
		    if (newField) {
			newField.setDisabled(false);
			newField.focus();
			newField.validate();
		    }

		    let confirmText = newCard.confirmText || gettext('Confirm Second Factor');
		    this.getViewModel().set('confirmText', confirmText);

		    this.saveLastTabUsed(tabPanel, newCard);
		},
	    },
	    'field': {
		validitychange: function(field, valid) {
		    // triggers only for enabled fields and we disable the one from the
		    // non-visible tab, so we can just directly use the valid param
		    this.getViewModel().set('canConfirm', valid);
		},
		afterrender: field => field.focus(), // ensure focus after initial render
	    },
	},

	saveLastTabUsed: function(tabPanel, card) {
	    let id = tabPanel.items.indexOf(card);
	    window.localStorage.setItem('PBS.TFALogin.lastTab', JSON.stringify({ id }));
	},

	getLastTabUsed: function() {
	    let data = window.localStorage.getItem('PBS.TFALogin.lastTab');
	    if (typeof data === 'string') {
		let last = JSON.parse(data);
		return last.id;
	    }
	    return null;
	},

	onClose: function() {
	    let me = this;
	    let view = me.getView();

	    if (!view.cancelled) {
		return;
	    }

	    view.onReject();
	},

	cancel: function() {
	    this.getView().close();
	},

	loginTotp: function() {
	    let me = this;

	    let code = me.lookup('totp').getValue();
	    let _promise = me.finishChallenge(`totp:${code}`);
	},

	loginWebauthn: async function() {
	    let me = this;
	    let view = me.getView();

	    me.lookup('webAuthnWaiting').setVisible(true);
	    me.lookup('webAuthnError').setVisible(false);

	    let challenge = view.challenge.webauthn;

	    if (typeof challenge.string !== 'string') {
		// Byte array fixup, keep challenge string:
		challenge.string = challenge.publicKey.challenge;
		challenge.publicKey.challenge = PBS.Utils.base64url_to_bytes(challenge.string);
		for (const cred of challenge.publicKey.allowCredentials) {
		    cred.id = PBS.Utils.base64url_to_bytes(cred.id);
		}
	    }

	    let controller = new AbortController();
	    challenge.signal = controller.signal;

	    let hwrsp;
	    try {
		//Promise.race( ...
		hwrsp = await navigator.credentials.get(challenge);
	    } catch (error) {
		// we do NOT want to fail login because of canceling the challenge actively,
		// in some browser that's the only way to switch over to another method as the
		// disallow user input during the time the challenge is active
		// checking for error.code === DOMException.ABORT_ERR only works in firefox -.-
		this.getViewModel().set('canConfirm', true);
		// FIXME: better handling, show some message, ...?
		me.lookup('webAuthnError').setData({
		    error: Ext.htmlEncode(error.toString()),
		});
		me.lookup('webAuthnError').setVisible(true);
		return;
	    } finally {
		let waitingMessage = me.lookup('webAuthnWaiting');
		if (waitingMessage) {
		    waitingMessage.setVisible(false);
		}
	    }

	    let response = {
		id: hwrsp.id,
		type: hwrsp.type,
		challenge: challenge.string,
		rawId: PBS.Utils.bytes_to_base64url(hwrsp.rawId),
		response: {
		    authenticatorData: PBS.Utils.bytes_to_base64url(
			hwrsp.response.authenticatorData,
		    ),
		    clientDataJSON: PBS.Utils.bytes_to_base64url(hwrsp.response.clientDataJSON),
		    signature: PBS.Utils.bytes_to_base64url(hwrsp.response.signature),
		},
	    };

	    await me.finishChallenge("webauthn:" + JSON.stringify(response));
	},

	loginRecovery: function() {
	    let me = this;

	    let key = me.lookup('recoveryKey').getValue();
	    let _promise = me.finishChallenge(`recovery:${key}`);
	},

	loginTFA: function() {
	    let me = this;
	    // avoid triggering more than once during challenge
	    me.getViewModel().set('canConfirm', false);
	    let view = me.getView();
	    let tfaPanel = view.down('tabpanel').getActiveTab();
	    me[tfaPanel.handler]();
	},

	finishChallenge: function(password) {
	    let me = this;
	    let view = me.getView();
	    view.cancelled = false;

	    let params = {
		username: view.userid,
		'tfa-challenge': view.ticket,
		password,
	    };

	    let resolve = view.onResolve;
	    let reject = view.onReject;
	    view.close();

	    return PBS.Async.api2({
		url: '/api2/extjs/access/ticket',
		method: 'POST',
		params,
	    })
	    .then(resolve)
	    .catch(reject);
	},
    },

    listeners: {
	close: 'onClose',
    },

    items: [{
	xtype: 'tabpanel',
	region: 'center',
	layout: 'fit',
	bodyPadding: 10,
	items: [
	    {
		xtype: 'panel',
		title: 'WebAuthn',
		iconCls: 'fa fa-fw fa-shield',
		confirmText: gettext('Start WebAuthn challenge'),
		handler: 'loginWebauthn',
		bind: {
		    disabled: '{!availableChallenge.webauthn}',
		},
		items: [
		    {
			xtype: 'box',
			html: gettext('Please insert your authentication device and press its button'),
		    },
		    {
			xtype: 'box',
			html: gettext('Waiting for second factor.') +`<i class="fa fa-refresh fa-spin fa-fw"></i>`,
			reference: 'webAuthnWaiting',
			hidden: true,
		    },
		    {
			xtype: 'box',
			data: {
			    error: '',
			},
			tpl: '<i class="fa fa-warning warning"></i> {error}',
			reference: 'webAuthnError',
			hidden: true,
		    },
		],
	    },
	    {
		xtype: 'panel',
		title: gettext('TOTP App'),
		iconCls: 'fa fa-fw fa-clock-o',
		handler: 'loginTotp',
		bind: {
		    disabled: '{!availableChallenge.totp}',
		},
		items: [
		    {
			xtype: 'textfield',
			fieldLabel: gettext('Please enter your TOTP verification code'),
			labelWidth: 300,
			name: 'totp',
			disabled: true,
			reference: 'totp',
			allowBlank: false,
			regex: /^[0-9]{6}$/,
			regexText: gettext('TOTP codes consist of six decimal digits'),
		    },
		],
	    },
	    {
		xtype: 'panel',
		title: gettext('Recovery Key'),
		iconCls: 'fa fa-fw fa-file-text-o',
		handler: 'loginRecovery',
		bind: {
		    disabled: '{!availableChallenge.recovery}',
		},
		items: [
		    {
			xtype: 'box',
			reference: 'availableRecovery',
			hidden: true,
		    },
		    {
			xtype: 'textfield',
			fieldLabel: gettext('Please enter one of your single-use recovery keys'),
			labelWidth: 300,
			name: 'recoveryKey',
			disabled: true,
			reference: 'recoveryKey',
			allowBlank: false,
			regex: /^[0-9a-f]{4}(-[0-9a-f]{4}){3}$/,
			regexText: gettext('Does not look like a valid recovery key'),
		    },
		    {
			xtype: 'box',
			reference: 'recoveryLow',
			hidden: true,
			html: '<i class="fa fa-exclamation-triangle warning"></i>'
			    + gettext('Less than {0} recovery keys available. Please generate a new set after login!'),
		    },
		],
	    },
	],
    }],

    buttons: [
	{
	    handler: 'loginTFA',
	    reference: 'tfaButton',
	    disabled: true,
	    bind: {
		text: '{confirmText}',
		disabled: '{!canConfirm}',
	    },
	},
    ],
});
