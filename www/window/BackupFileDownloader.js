Ext.define('PBS.window.BackupFileDownloader', {
    extend: 'Ext.window.Window',
    alias: 'widget.pbsBackupFileDownloader',

    title: gettext('Download Files'),
    bodyPadding: 10,

    width: 400,
    modal: true,
    resizable: false,

    layout: {
	type: 'vbox',
	align: 'stretch',
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	buildUrl: function(baseurl, params) {
	    let url = new URL(baseurl, window.location.origin);
	    for (const [key, value] of Object.entries(params)) {
		url.searchParams.append(key, value);
	    }

	    return url.href;
	},

	downloadFile: function() {
	    let me = this;
	    let view = me.getView();
	    let combo = me.lookup('file');
	    let file = combo.getValue();

	    let idx = file.lastIndexOf('.');
	    let filename = file.slice(0, idx);
	    let atag = document.createElement('a');
	    let params = view.params;
	    params['file-name'] = file;
	    atag.download = filename;
	    atag.href = me.buildUrl(`${view.baseurl}/download-decoded`, params);
	    atag.click();
	},

	changeFile: function(comob, value) {
	    let me = this;
	    let combo = me.lookup('file');
	    let rec = combo.getStore().findRecord('filename', value, 0, false, true, true);
	    let canDownload = rec.data['crypt-mode'] !== 'encrypt';
	    me.lookup('encryptedHint').setVisible(!canDownload);
	    me.lookup('signedHint').setVisible(rec.data['crypt-mode'] === 'sign-only');
	    me.lookup('downloadBtn').setDisabled(!canDownload);
	},

	init: function(view) {
	    let me = this;
	    if (!view.baseurl) {
		throw "no baseurl given";
	    }

	    if (!view.params) {
		throw "no params given";
	    }

	    if (!view.files) {
		throw "no files given";
	    }

	    me.lookup('file').getStore().loadData(view.files, false);
	},

	control: {
	    'proxmoxComboGrid': {
		change: 'changeFile',
	    },
	    'button': {
		click: 'downloadFile',
	    },
	},
    },

    items: [
	{
	    xtype: 'proxmoxComboGrid',
	    valueField: 'filename',
	    allowBlank: false,
	    displayField: 'filename',
	    reference: 'file',
	    emptyText: gettext('No file selected'),
	    fieldLabel: gettext('File'),
	    store: {
		fields: ['filename', 'size', 'crypt-mode'],
		idProperty: ['filename'],
	    },
	    listConfig: {
		emptyText: gettext('No Data'),
		columns: [
		    {
			text: gettext('Filename'),
			dataIndex: 'filename',
			renderer: Ext.String.htmlEncode,
			flex: 1,
		    },
		    {
			text: gettext('Size'),
			dataIndex: 'size',
			renderer: val => val === undefined ? '' : Proxmox.Utils.format_size(val),
		    },
		    {
			text: gettext('Encrypted'),
			dataIndex: 'crypt-mode',
			renderer: function(value) {
			    let mode = -1;
			    if (value !== undefined) {
				mode = PBS.Utils.cryptmap.indexOf(value);
			    }
			    return PBS.Utils.cryptText[mode] || Proxmox.Utils.unknownText;
			},
		    },
		],
	    },
	},
	{
	    xtype: 'displayfield',
	    userCls: 'pmx-hint',
	    reference: 'signedHint',
	    hidden: true,
	    value: gettext('Note: Signatures of signed files will not be verified on the server. Please use the client to do this.'),
	},
	{
	    xtype: 'displayfield',
	    userCls: 'pmx-hint',
	    reference: 'encryptedHint',
	    hidden: true,
	    value: gettext('Encrypted Files cannot be decoded on the server directly. Please use the client where the decryption key is located.'),
	},
    ],

    buttons: [
	{
	    text: gettext('Download'),
	    reference: 'downloadBtn',
	    disabled: true,
	},
    ],
});
