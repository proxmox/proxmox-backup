Ext.define('PBS.TapeManagement.EncryptionEditWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsEncryptionEditWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    isCreate: true,
    isAdd: true,
    subject: gettext('Encryption Key'),
    cbindData: function(initialConfig) {
	let me = this;

	let fingerprint = initialConfig.fingerprint;
	let baseurl = '/api2/extjs/config/tape-encryption-keys';

	me.isCreate = !fingerprint;
	me.url = fingerprint ? `${baseurl}/${encodeURIComponent(fingerprint)}` : baseurl;
	me.method = fingerprint ? 'PUT' : 'POST';

	return { };
    },

    items: [
	{
	    fieldLabel: gettext('Hint'),
	    name: 'hint',
	    xtype: 'pmxDisplayEditField',
	    renderer: Ext.htmlEncode,
	    allowBlank: false,
	    cbind: {
		editable: '{isCreate}',
	    },
	},
	{
	    xtype: 'textfield',
	    inputType: 'password',
	    fieldLabel: gettext('Password'),
	    name: 'password',
	    minLength: 5,
	    allowBlank: false,
	},
	{
	    xtype: 'textfield',
	    inputType: 'password',
	    submitValue: false,
	    fieldLabel: gettext('Confirm Password'),
	    minLength: 5,
	    vtype: 'password',
	    initialPassField: 'password',
	    allowBlank: false,
	},
    ],
});
