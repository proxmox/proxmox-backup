Ext.define('PBS.window.AddWebauthn', {
    extend: 'Ext.window.Window',
    alias: 'widget.pbsAddWebauthn',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_mgmt',

    modal: true,
    resizable: false,
    title: gettext('Add a Webauthn login token'),
    width: 512,

    user: undefined,
    fixedUser: false,

    initComponent: function() {
	let me = this;
	me.callParent();
	Ext.GlobalEvents.fireEvent('proxmoxShowHelp', me.onlineHelp);
    },

    viewModel: {
	data: {
	    valid: false,
	    userid: null,
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	control: {
	    'field': {
		validitychange: function(field, valid) {
		    let me = this;
		    let viewmodel = me.getViewModel();
		    let form = me.lookup('webauthn_form');
		    viewmodel.set('valid', form.isValid());
		},
	    },
	    '#': {
		show: function() {
		    let me = this;
		    let view = me.getView();

		    if (Proxmox.UserName === 'root@pam') {
			view.lookup('password').setVisible(false);
			view.lookup('password').setDisabled(true);
		    }
		},
	    },
	},

	registerWebauthn: async function() {
	    let me = this;
	    let values = me.lookup('webauthn_form').getValues();
	    values.type = "webauthn";

	    let userid = values.user;
	    delete values.user;

	    me.getView().mask(gettext('Please wait...'), 'x-mask-loading');

	    try {
		let register_response = await Proxmox.Async.api2({
		    url: `/api2/extjs/access/tfa/${userid}`,
		    method: 'POST',
		    params: values,
		});

		let data = register_response.result.data;
		if (!data.challenge) {
		    throw "server did not respond with a challenge";
		}

		let creds = JSON.parse(data.challenge);

		// Fix this up before passing it to the browser, but keep a copy of the original
		// string to pass in the response:
		let challenge_str = creds.publicKey.challenge;
		creds.publicKey.challenge = PBS.Utils.base64url_to_bytes(challenge_str);
		creds.publicKey.user.id =
		    PBS.Utils.base64url_to_bytes(creds.publicKey.user.id);

		// convert existing authenticators structure
		creds.publicKey.excludeCredentials =
		    (creds.publicKey.excludeCredentials || [])
		    .map((credential) => ({
			id: PBS.Utils.base64url_to_bytes(credential.id),
			type: credential.type,
		    }));

		let msg = Ext.Msg.show({
		    title: `Webauthn: ${gettext('Setup')}`,
		    message: gettext('Please press the button on your Webauthn Device'),
		    buttons: [],
		});

		let token_response;
		try {
		    token_response = await navigator.credentials.create(creds);
		} catch (error) {
		    let errmsg = error.message;
		    if (error.name === 'InvalidStateError') {
			errmsg = gettext('Is this token already registered?');
		    }
		    throw gettext('An error occurred during token registration.') +
			`<br>${error.name}: ${errmsg}`;
		}

		// We cannot pass ArrayBuffers to the API, so extract & convert the data.
		let response = {
		    id: token_response.id,
		    type: token_response.type,
		    rawId: PBS.Utils.bytes_to_base64url(token_response.rawId),
		    response: {
			attestationObject: PBS.Utils.bytes_to_base64url(
			    token_response.response.attestationObject,
			),
			clientDataJSON: PBS.Utils.bytes_to_base64url(
			    token_response.response.clientDataJSON,
			),
		    },
		};

		msg.close();

		let params = {
		    type: "webauthn",
		    challenge: challenge_str,
		    value: JSON.stringify(response),
		};

		if (values.password) {
		    params.password = values.password;
		}

		await Proxmox.Async.api2({
		    url: `/api2/extjs/access/tfa/${userid}`,
		    method: 'POST',
		    params,
		});
	    } catch (response) {
		let error = response.result.message;
		console.error(error); // for debugging if it's not displayable...
		Ext.Msg.alert(gettext('Error'), error);
	    }

	    me.getView().close();
	},
    },

    items: [
	{
	    xtype: 'form',
	    reference: 'webauthn_form',
	    layout: 'anchor',
	    border: false,
	    bodyPadding: 10,
	    fieldDefaults: {
		anchor: '100%',
	    },
	    items: [
		{
		    xtype: 'pmxDisplayEditField',
		    name: 'user',
		    cbind: {
			editable: (get) => !get('fixedUser'),
			value: () => Proxmox.UserName,
		    },
		    fieldLabel: gettext('User'),
		    editConfig: {
			xtype: 'pbsUserSelector',
			allowBlank: false,
		    },
		    renderer: Ext.String.htmlEncode,
		    listeners: {
			change: function(field, newValue, oldValue) {
			    let vm = this.up('window').getViewModel();
			    vm.set('userid', newValue);
			},
		    },
		},
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Description'),
		    allowBlank: false,
		    name: 'description',
		    maxLength: 256,
		    emptyText: gettext('For example: TFA device ID, required to identify multiple factors.'),
		},
		{
		    xtype: 'textfield',
		    name: 'password',
		    reference: 'password',
		    fieldLabel: gettext('Verify Password'),
		    inputType: 'password',
		    minLength: 5,
		    allowBlank: false,
		    validateBlank: true,
		    cbind: {
			hidden: () => Proxmox.UserName === 'root@pam',
			disabled: () => Proxmox.UserName === 'root@pam',
			emptyText: () =>
			    Ext.String.format(gettext("Confirm your ({0}) password"), Proxmox.UserName),
		    },
		},
	    ],
	},
    ],

    buttons: [
	{
	    xtype: 'proxmoxHelpButton',
	},
	'->',
	{
	    xtype: 'button',
	    text: gettext('Register Webauthn Device'),
	    handler: 'registerWebauthn',
	    bind: {
		disabled: '{!valid}',
	    },
	},
    ],
});
