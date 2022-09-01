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
		    let resp = await Proxmox.Async.api2({
			url: '/api2/extjs/access/openid/auth-url',
			params: {
			    realm: creds.realm,
			    "redirect-url": redirectURL,
			},
			method: 'POST',
		    });
		    window.location = resp.result.data;
		} catch (response) {
		    Proxmox.Utils.authClear();
		    loginForm.unmask();
		    Ext.MessageBox.alert(
			gettext('Error'),
			gettext('OpenID redirect failed, please try again') + `<br>${response.result.message}`,
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
		let resp = await Proxmox.Async.api2({
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
		Ext.create('Proxmox.window.TfaLoginWindow', {
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
			    value: Ext.util.Cookies.get('PBSLangCookie') || Proxmox.defaultLang || '__default__',
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
