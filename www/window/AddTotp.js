/*global QRCode*/
Ext.define('PBS.window.AddTotp', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsAddTotp',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_mgmt',

    modal: true,
    resizable: false,
    title: gettext('Add a TOTP login factor'),
    width: 512,
    layout: {
	type: 'vbox',
	align: 'stretch',
    },

    isAdd: true,
    userid: undefined,
    tfa_id: undefined,
    fixedUser: false,

    updateQrCode: function() {
	let me = this;
	let values = me.lookup('totp_form').getValues();
	let algorithm = values.algorithm;
	if (!algorithm) {
	    algorithm = 'SHA1';
	}

	let otpuri =
	    'otpauth://totp/' + encodeURIComponent(values.userid) +
	    '?secret=' + values.secret +
	    '&period=' + values.step +
	    '&digits=' + values.digits +
	    '&algorithm=' + algorithm +
	    '&issuer=' + encodeURIComponent(values.issuer);

	me.getController().getViewModel().set('otpuri', otpuri);
	me.qrcode.makeCode(otpuri);
	me.lookup('challenge').setVisible(true);
	me.down('#qrbox').setVisible(true);
    },

    viewModel: {
	data: {
	    valid: false,
	    secret: '',
	    otpuri: '',
	},

	formulas: {
	    secretEmpty: function(get) {
		return get('secret').length === 0;
	    },
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',
	control: {
	    'field[qrupdate=true]': {
		change: function() {
		    this.getView().updateQrCode();
		},
	    },
	    'field': {
		validitychange: function(field, valid) {
		    let me = this;
		    let viewModel = me.getViewModel();
		    let form = me.lookup('totp_form');
		    let challenge = me.lookup('challenge');
		    let password = me.lookup('password');
		    viewModel.set('valid', form.isValid() && challenge.isValid() && password.isValid());
		},
	    },
	    '#': {
		show: function() {
		    let me = this;
		    let view = me.getView();

		    view.qrdiv = document.createElement('div');
		    view.qrcode = new QRCode(view.qrdiv, {
			width: 256,
			height: 256,
			correctLevel: QRCode.CorrectLevel.M,
		    });
		    view.down('#qrbox').getEl().appendChild(view.qrdiv);

		    view.getController().randomizeSecret();

		    if (Proxmox.UserName === 'root@pam') {
			view.lookup('password').setVisible(false);
			view.lookup('password').setDisabled(true);
		    }
		},
	    },
	},

	randomizeSecret: function() {
	    let me = this;
	    let rnd = new Uint8Array(32);
	    window.crypto.getRandomValues(rnd);
	    let data = '';
	    rnd.forEach(function(b) {
		// secret must be base32, so just use the first 5 bits
		b = b & 0x1f;
		if (b < 26) {
		    // A..Z
		    data += String.fromCharCode(b + 0x41);
		} else {
		    // 2..7
		    data += String.fromCharCode(b-26 + 0x32);
		}
	    });
	    me.getViewModel().set('secret', data);
	},
    },

    items: [
	{
	    xtype: 'form',
	    layout: 'anchor',
	    border: false,
	    reference: 'totp_form',
	    fieldDefaults: {
		anchor: '100%',
	    },
	    items: [
		{
		    xtype: 'pmxDisplayEditField',
		    name: 'userid',
		    cbind: {
			editable: (get) => get('isAdd') && !get('fixedUser'),
		    },
		    fieldLabel: gettext('User'),
		    editConfig: {
			xtype: 'pbsUserSelector',
			allowBlank: false,
		    },
		    renderer: Ext.String.htmlEncode,
		    value: Proxmox.UserName,
		    qrupdate: true,
		},
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Description'),
		    allowBlank: false,
		    name: 'description',
		    maxLength: 256,
		},
		{
		    layout: 'hbox',
		    border: false,
		    padding: '0 0 5 0',
		    items: [
			{
			    xtype: 'textfield',
			    fieldLabel: gettext('Secret'),
			    emptyText: gettext('Unchanged'),
			    name: 'secret',
			    reference: 'tfa_secret',
			    regex: /^[A-Z2-7=]+$/,
			    regexText: 'Must be base32 [A-Z2-7=]',
			    maskRe: /[A-Z2-7=]/,
			    qrupdate: true,
			    bind: {
				value: "{secret}",
			    },
			    flex: 4,
			    padding: '0 5 0 0',
			},
			{
			    xtype: 'button',
			    text: gettext('Randomize'),
			    reference: 'randomize_button',
			    handler: 'randomizeSecret',
			    flex: 1,
			},
		    ],
		},
		{
		    xtype: 'numberfield',
		    fieldLabel: gettext('Time period'),
		    name: 'step',
		    // Google Authenticator ignores this and generates bogus data
		    hidden: true,
		    value: 30,
		    minValue: 10,
		    qrupdate: true,
		},
		{
		    xtype: 'numberfield',
		    fieldLabel: gettext('Digits'),
		    name: 'digits',
		    value: 6,
		    // Google Authenticator ignores this and generates bogus data
		    hidden: true,
		    minValue: 6,
		    maxValue: 8,
		    qrupdate: true,
		},
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Issuer Name'),
		    name: 'issuer',
		    value: `Proxmox Backup Server - ${Proxmox.NodeName}`,
		    qrupdate: true,
		},
		{
		    xtype: 'box',
		    itemId: 'qrbox',
		    visible: false, // will be enabled when generating a qr code
		    bind: {
			visible: '{!secretEmpty}',
		    },
		    style: {
			'background-color': 'white',
			'margin-left': 'auto',
			'margin-right': 'auto',
			padding: '5px',
			width: '266px',
			height: '266px',
		    },
		},
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Verification Code'),
		    allowBlank: false,
		    reference: 'challenge',
		    name: 'challenge',
		    bind: {
			disabled: '{!showTOTPVerifiction}',
			visible: '{showTOTPVerifiction}',
		    },
		    emptyText: gettext('Scan QR code and enter TOTP auth. code to verify'),
		},
		{
		    xtype: 'textfield',
		    inputType: 'password',
		    fieldLabel: gettext('Password'),
		    minLength: 5,
		    reference: 'password',
		    name: 'password',
		    allowBlank: false,
		    validateBlank: true,
		    emptyText: gettext('verify current password'),
		},
	    ],
	},
    ],

    initComponent: function() {
	let me = this;
	me.url = '/api2/extjs/access/tfa/';
	me.method = 'POST';
	me.callParent();
    },

    getValues: function(dirtyOnly) {
	let me = this;
	let viewmodel = me.getController().getViewModel();

	let values = me.callParent(arguments);

	let uid = encodeURIComponent(values.userid);
	me.url = `/api2/extjs/access/tfa/${uid}`;
	delete values.userid;

	let data = {
	    description: values.description,
	    type: "totp",
	    totp: viewmodel.get('otpuri'),
	    value: values.challenge,
	};

	if (values.password) {
	    data.password = values.password;
	}

	return data;
    },
});
