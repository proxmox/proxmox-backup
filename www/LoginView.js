Ext.define('PBS.LoginView', {
    extend: 'Ext.container.Container',
    xtype: 'loginview',

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

	    let params = loginForm.getValues();

	    params.username = params.username + '@' + params.realm;
	    delete params.realm;

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
		    params: params,
		    method: 'POST',
		});

		let data = resp.result.data;
		if (data.ticket.startsWith("PBS:!tfa!")) {
		    data = await me.performTFAChallenge(data);
		}

		PBS.Utils.updateLoginData(data);
		PBS.app.changeView('mainview');
	    } catch (error) {
		console.error(error); // for debugging
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
			},
			{
			    xtype: 'textfield',
			    inputType: 'password',
			    fieldLabel: gettext('Password'),
			    name: 'password',
			    itemId: 'passwordField',
			    reference: 'passwordField',
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
			},
			{
			    text: gettext('Login'),
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

    modal: true,
    resizable: false,
    title: gettext("Second login factor required"),

    cancelled: true,

    width: 512,
    layout: {
	type: 'vbox',
	align: 'stretch',
    },

    defaultButton: 'totpButton',

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    let me = this;

	    if (!view.userid) {
		throw "no userid given";
	    }

	    if (!view.ticket) {
		throw "no ticket given";
	    }

	    if (!view.challenge) {
		throw "no challenge given";
	    }

	    if (!view.challenge.webauthn) {
		me.lookup('webauthnButton').setVisible(false);
	    }

	    if (!view.challenge.totp) {
		me.lookup('totpButton').setVisible(false);
	    }

	    if (!view.challenge.recovery) {
		me.lookup('recoveryButton').setVisible(false);
	    } else if (view.challenge.recovery === "low") {
		me.lookup('recoveryButton')
		    .setIconCls('fa fa-fw fa-exclamation-triangle');
	    }


	    if (!view.challenge.totp && !view.challenge.recovery) {
		// only webauthn tokens available, maybe skip ahead?
		me.lookup('totp').setVisible(false);
		me.lookup('waiting').setVisible(true);
		let _promise = me.loginWebauthn();
	    }
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

	    let _promise = me.finishChallenge('totp:' + me.lookup('totp').value);
	},

	loginWebauthn: async function() {
	    let me = this;
	    let view = me.getView();

	    // avoid this window ending up above the tfa popup if we got triggered from init().
	    await PBS.Async.sleep(100);

	    // FIXME: With webauthn the browser provides a popup (since it doesn't necessarily need
	    // to require pressing a button, but eg. use a fingerprint scanner or face detection
	    // etc., so should we just trust that that happens and skip the popup?)
	    let msg = Ext.Msg.show({
		title: `Webauthn: ${gettext('Login')}`,
		message: gettext('Please press the button on your Authenticator Device'),
		buttons: [],
	    });

	    let challenge = view.challenge.webauthn;

	    // Byte array fixup, keep challenge string:
	    let challenge_str = challenge.publicKey.challenge;
	    challenge.publicKey.challenge = PBS.Utils.base64url_to_bytes(challenge_str);
	    for (const cred of challenge.publicKey.allowCredentials) {
		cred.id = PBS.Utils.base64url_to_bytes(cred.id);
	    }

	    let hwrsp;
	    try {
		hwrsp = await navigator.credentials.get(challenge);
	    } catch (error) {
		view.onReject(error);
		return;
	    } finally {
		msg.close();
	    }

	    let response = {
		id: hwrsp.id,
		type: hwrsp.type,
		challenge: challenge_str,
		rawId: PBS.Utils.bytes_to_base64url(hwrsp.rawId),
		response: {
		    authenticatorData: PBS.Utils.bytes_to_base64url(
			hwrsp.response.authenticatorData,
		    ),
		    clientDataJSON: PBS.Utils.bytes_to_base64url(hwrsp.response.clientDataJSON),
		    signature: PBS.Utils.bytes_to_base64url(hwrsp.response.signature),
		},
	    };

	    msg.close();

	    await me.finishChallenge("webauthn:" + JSON.stringify(response));
	},

	loginRecovery: function() {
	    let me = this;
	    let view = me.getView();

	    if (me.login_recovery_confirm) {
		let _promise = me.finishChallenge('recovery:' + me.lookup('totp').value);
	    } else {
		me.login_recovery_confirm = true;
		me.lookup('totpButton').setVisible(false);
		me.lookup('webauthnButton').setVisible(false);
		me.lookup('recoveryButton').setText(gettext("Confirm"));
		me.lookup('recoveryInfo').setVisible(true);
		if (view.challenge.recovery === "low") {
		    me.lookup('recoveryLow').setVisible(true);
		}
	    }
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

    items: [
	{
	    xtype: 'form',
	    layout: 'anchor',
	    border: false,
	    fieldDefaults: {
		anchor: '100%',
		padding: '0 5',
	    },
	    items: [
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Please enter your OTP verification code:'),
		    labelWidth: '300px',
		    name: 'totp',
		    reference: 'totp',
		    allowBlank: false,
		},
	    ],
	},
	{
	    xtype: 'box',
	    html: gettext('Waiting for second factor.'),
	    reference: 'waiting',
	    padding: '0 5',
	    hidden: true,
	},
	{
	    xtype: 'box',
	    padding: '0 5',
	    reference: 'recoveryInfo',
	    hidden: true,
	    html: gettext('Please note that each recovery code can only be used once!'),
	    style: {
		textAlign: "center",
	    },
	},
	{
	    xtype: 'box',
	    padding: '0 5',
	    reference: 'recoveryLow',
	    hidden: true,
	    html: '<i class="fa fa-exclamation-triangle warning"></i>'
		+ gettext('Only few recovery keys available. Please generate a new set!')
		+ '<i class="fa fa-exclamation-triangle warning"></i>',
	    style: {
		textAlign: "center",
	    },
	},
    ],

    buttons: [
	{
	    text: gettext('Login with TOTP'),
	    handler: 'loginTotp',
	    reference: 'totpButton',
	},
	{
	    text: gettext('Login with a recovery key'),
	    handler: 'loginRecovery',
	    reference: 'recoveryButton',
	},
	{
	    text: gettext('Use a Webauthn token'),
	    handler: 'loginWebauthn',
	    reference: 'webauthnButton',
	},
    ],
});
