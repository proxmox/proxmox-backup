Ext.define('PBS.LoginView', {
    extend: 'Ext.container.Container',
    xtype: 'loginview',

    controller: {
	xclass: 'Ext.app.ViewController',

	submitForm: function() {
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

	    Proxmox.Utils.API2Request({
		url: '/api2/extjs/access/ticket',
		params: params,
		method: 'POST',
		success: function(resp, opts) {
		    // save login data and create cookie
		    PBS.Utils.updateLoginData(resp.result.data);
		    PBS.app.changeView('mainview');
		},
		failure: function(resp, opts) {
		    Proxmox.Utils.authClear();
		    loginForm.unmask();
		    Ext.MessageBox.alert(
			gettext('Error'),
			gettext('Login failed. Please try again'),
		    );
		},
	    });
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
