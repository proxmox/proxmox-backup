Ext.define('PBS.LoginView', {
    extend: 'Ext.container.Container',
    xtype: 'loginview',

    controller: {
	xclass: 'Ext.app.ViewController',

	submitForm: function() {
	    var me = this;
	    var view = me.getView();
	    var loginForm = me.lookupReference('loginForm');

	    if (loginForm.isValid()) {
		if (loginForm.isVisible()) {
		    loginForm.mask(gettext('Please wait...'), 'x-mask-loading');
		}
		loginForm.submit({
		    success: function(form, action) {
			// save login data and create cookie
			PBS.Utils.updateLoginData(action.result.data);
			PBS.app.changeView('mainview');
		    },
		    failure: function(form, action) {
			loginForm.unmask();
			Ext.MessageBox.alert(
			    gettext('Error'),
			    gettext('Login failed. Please try again')
			);
		    }
		});
	    }
	},

	control: {
	    'button[reference=loginButton]': {
		click: 'submitForm'
	    }
	}
    },

    plugins: 'viewport',

    layout: {
	type: 'border'
    },

    items: [
	{
	    region: 'north',
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'middle'
	    },
	    margin: '2 5 2 5',
	    height: 38,
	    items: [
		{
		    xtype: 'proxmoxlogo'
		},
		{
		    xtype: 'versioninfo',
		    makeApiCall: false
		}
	    ]
	},
	{
	    region: 'center'
	},
	{
	    xtype: 'window',
	    closable: false,
	    resizable: false,
	    reference: 'loginwindow',
	    autoShow: true,
	    modal: true,

	    //defaultFocus: 'usernameField',
	    // TODO: use usernameField again once we have a real user-,
	    // permission system and root@pam isn't the default anymore
	    defaultFocus: 'passwordField',

	    layout: {
		type: 'auto'
	    },

	    title: gettext('Proxmox Backup Server Login'),

	    items: [
		{
		    xtype: 'form',
		    layout: {
			type: 'form'
		    },
		    defaultButton: 'loginButton',
		    url: '/api2/extjs/access/ticket',
		    reference: 'loginForm',

		    fieldDefaults: {
			labelAlign: 'right',
			allowBlank: false
		    },

		    items: [
			{
			    xtype: 'textfield',
			    fieldLabel: gettext('User name'),
			    name: 'username',
			    value: 'root@pam',
			    itemId: 'usernameField',
			    reference: 'usernameField'
			},
			{
			    xtype: 'textfield',
			    inputType: 'password',
			    fieldLabel: gettext('Password'),
			    name: 'password',
			    itemId: 'passwordField',
			    reference: 'passwordField',
			}
		    ],
		    buttons: [
			{
			    text: gettext('Login'),
			    reference: 'loginButton',
			    formBind: true
			}
		    ]
		}
	    ]
	}
    ]
});
