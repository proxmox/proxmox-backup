Ext.define('PBS.window.DatastoreRepoInfo', {
    extend: 'Ext.window.Window',
    alias: 'widget.pbsDatastoreRepoInfo',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext('Connection Information'),

    modal: true,
    resizable: false,
    width: 600,
    layout: 'anchor',
    bodyPadding: 10,

    cbindData: function() {
	let me = this;
	let fingerprint = Proxmox.Fingerprint;
	let host = window.location.hostname;
	let hostname = host;
	if (window.location.port.toString() !== "8007") {
	    host += `:${window.location.port}`;
	}
	let datastore = me.datastore;
	let user = Proxmox.UserName;
	let repository = `${host}:${datastore}`;
	let repositoryWithUser = `${user}@${host}:${datastore}`;

	return {
	    datastore,
	    hostname,
	    fingerprint,
	    repository,
	    repositoryWithUser,
	};
    },

    defaults: {
	xtype: 'pbsCopyField',
	labelWidth: 120,
    },

    items: [
	{
	    fieldLabel: gettext('Datastore'),
	    cbind: {
		value: '{datastore}',
	    },
	},
	{
	    fieldLabel: gettext('Hostname/IP'),
	    cbind: {
		value: '{hostname}',
	    },
	},
	{
	    fieldLabel: gettext('Fingerprint'),
	    cbind: {
		value: '{fingerprint}',
		hidden: '{!fingerprint}',
	    },
	},
	{
	    xtype: 'displayfield',
	    value: '',
	    labelWidth: 500,
	    fieldLabel: gettext('Repository for CLI and API'),
	    padding: '10 0 0 0',
	},
	{
	    fieldLabel: gettext('Repository'),
	    cbind: {
		value: '{repository}',
	    },
	},
	{
	    fieldLabel: gettext('With Current User'),
	    cbind: {
		value: '{repositoryWithUser}',
	    },
	},
    ],
    buttons: [
	{
	    xtype: 'proxmoxHelpButton',
	    onlineHelp: 'client_repository',
	    hidden: false,
	},
	'->',
	{
	    text: gettext('Ok'),
	    handler: function() {
		this.up('window').close();
	    },
	},
    ],
});

Ext.define('PBS.form.CopyField', {
    extend: 'Ext.form.FieldContainer',
    alias: 'widget.pbsCopyField',

    layout: 'hbox',

    items: [
	{
	    xtype: 'textfield',
	    itemId: 'inputField',
	    editable: false,
	    flex: 1,
	},
	{
	    xtype: 'button',
	    margin: '0 0 0 10',
	    iconCls: 'fa fa-clipboard x-btn-icon-el-default-toolbar-small',
	    baseCls: 'x-btn',
	    cls: 'x-btn-default-toolbar-small proxmox-inline-button',
	    handler: function() {
		let me = this;
		let field = me.up('pbsCopyField');
		let el = field.getComponent('inputField')?.inputEl;
		if (!el?.dom) {
		    return;
		}
		el.dom.select();
		document.execCommand("copy");
	    },
	    text: gettext('Copy'),
	},
    ],

    initComponent: function() {
	let me = this;
	me.callParent();
	me.getComponent('inputField').setValue(me.value);
    },
});
