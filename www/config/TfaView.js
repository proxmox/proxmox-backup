Ext.define('pbs-tfa-users', {
    extend: 'Ext.data.Model',
    fields: ['userid'],
    idProperty: 'userid',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/access/tfa',
    },
});

Ext.define('pbs-tfa-entry', {
    extend: 'Ext.data.Model',
    fields: ['fullid', 'userid', 'type', 'description', 'created', 'enable'],
    idProperty: 'fullid',
});


Ext.define('PBS.config.TfaView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsTfaView',

    title: gettext('Second Factors'),
    reference: 'tfaview',

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	model: 'pbs-tfa-entry',
	rstore: {
	    type: 'store',
	    proxy: 'memory',
	    storeid: 'pbs-tfa-entry',
	    model: 'pbs-tfa-entry',
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    let me = this;
	    view.tfaStore = Ext.create('Proxmox.data.UpdateStore', {
		autoStart: true,
		interval: 5 * 1000,
		storeid: 'pbs-tfa-users',
		model: 'pbs-tfa-users',
	    });
	    view.tfaStore.on('load', this.onLoad, this);
	    view.on('destroy', view.tfaStore.stopUpdate);
	    Proxmox.Utils.monStoreErrors(view, view.tfaStore);
	},

	reload: function() { this.getView().tfaStore.load(); },

	onLoad: function(store, data, success) {
	    if (!success) return;

	    let records = [];
	    Ext.Array.each(data, user => {
		Ext.Array.each(user.data.entries, entry => {
		    records.push({
			fullid: `${user.id}/${entry.id}`,
			userid: user.id,
			type: entry.type,
			description: entry.description,
			created: entry.created,
			enable: entry.enable,
		    });
		});
	    });

	    let rstore = this.getView().store.rstore;
	    rstore.loadData(records);
	    rstore.fireEvent('load', rstore, records, true);
	},

	addTotp: function() {
	    let me = this;

	    Ext.create('PBS.window.AddTotp', {
		isCreate: true,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	addWebauthn: function() {
	    let me = this;

	    Ext.create('PBS.window.AddWebauthn', {
		isCreate: true,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	addRecovery: async function() {
	    let me = this;

	    Ext.create('PBS.window.AddTfaRecovery', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	editItem: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length !== 1 || selection[0].id.endsWith("/recovery")) {
		return;
	    }

	    Ext.create('PBS.window.TfaEdit', {
		'tfa-id': selection[0].data.fullid,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	renderUser: fullid => fullid.split('/')[0],

	renderEnabled: enabled => {
	    if (enabled === undefined) {
		return Proxmox.Utils.yesText;
	    } else {
		return Proxmox.Utils.format_boolean(enabled);
	    }
	},

	onRemoveButton: function(btn, event, record) {
	    let me = this;

	    Ext.create('PBS.tfa.confirmRemove', {
		...record.data,
		callback: password => me.removeItem(password, record),
	    })
	    .show();
	},

	removeItem: async function(password, record) {
	    let me = this;

	    let params = {};
	    if (password !== null) {
		params.password = password;
	    }

	    try {
		me.getView().mask(gettext('Please wait...'), 'x-mask-loading');
		await Proxmox.Async.api2({
		    url: `/api2/extjs/access/tfa/${record.id}`,
		    method: 'DELETE',
		    params,
		});
		me.reload();
	    } catch (response) {
		Ext.Msg.alert(gettext('Error'), response.result.message);
	    } finally {
		me.getView().unmask();
            }
	},
    },

    viewConfig: {
	trackOver: false,
    },

    listeners: {
	itemdblclick: 'editItem',
    },

    columns: [
	{
	    header: gettext('User'),
	    width: 200,
	    sortable: true,
	    dataIndex: 'fullid',
	    renderer: 'renderUser',
	},
	{
	    header: gettext('Enabled'),
	    width: 80,
	    sortable: true,
	    dataIndex: 'enable',
	    renderer: 'renderEnabled',
	},
	{
	    header: gettext('TFA Type'),
	    width: 80,
	    sortable: true,
	    dataIndex: 'type',
	},
	{
	    header: gettext('Created'),
	    width: 150,
	    sortable: true,
	    dataIndex: 'created',
	    renderer: Proxmox.Utils.render_timestamp,
	},
	{
	    header: gettext('Description'),
	    width: 300,
	    sortable: true,
	    dataIndex: 'description',
	    renderer: Ext.String.htmlEncode,
	    flex: 1,
	},
    ],

    tbar: [
	{
	    text: gettext('Add'),
	    menu: {
		xtype: 'menu',
		items: [
		    {
			text: gettext('TOTP'),
			itemId: 'totp',
			iconCls: 'fa fa-fw fa-clock-o',
			handler: 'addTotp',
		    },
		    {
			text: gettext('Webauthn'),
			itemId: 'webauthn',
			iconCls: 'fa fa-fw fa-shield',
			handler: 'addWebauthn',
		    },
		    {
			text: gettext('Recovery Keys'),
			itemId: 'recovery',
			iconCls: 'fa fa-fw fa-file-text-o',
			handler: 'addRecovery',
		    },
		],
	    },
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editItem',
	    enableFn: rec => !rec.id.endsWith("/recovery"),
	    disabled: true,
	},
	{
	    xtype: 'proxmoxButton',
	    disabled: true,
	    text: gettext('Remove'),
	    getRecordName: rec => rec.data.description,
	    handler: 'onRemoveButton',
	},
    ],
});

Ext.define('PBS.tfa.confirmRemove', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext("Confirm TFA Removal"),

    modal: true,
    resizable: false,
    width: 600,
    isCreate: true, // logic
    isRemove: true,

    url: '/access/tfa',

    initComponent: function() {
	let me = this;

	if (typeof me.type !== "string") {
	    throw "missing type";
	}

	if (!me.callback) {
	    throw "missing callback";
	}

	me.callParent();

	if (Proxmox.UserName === 'root@pam') {
	    me.lookup('password').setVisible(false);
	    me.lookup('password').setDisabled(true);
	}
    },

    submit: function() {
	let me = this;
	if (Proxmox.UserName === 'root@pam') {
	    me.callback(null);
	} else {
	    me.callback(me.lookup('password').getValue());
	}
	me.close();
    },

    items: [
	{
	    xtype: 'box',
	    padding: '0 0 10 0',
	    html: Ext.String.format(
	        gettext('Are you sure you want to remove this {0} entry?'),
	        'TFA',
	    ),
	},
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'begin',
	    },
	    defaults: {
		border: false,
		layout: 'anchor',
		flex: 1,
		padding: 5,
	    },
	    items: [
		{
		    xtype: 'container',
		    layout: {
			type: 'vbox',
		    },
		    padding: '0 10 0 0',
		    items: [
			{
			    xtype: 'displayfield',
			    fieldLabel: gettext('User'),
			    cbind: {
				value: '{userid}',
			    },
			},
			{
			    xtype: 'displayfield',
			    fieldLabel: gettext('Type'),
			    cbind: {
				value: '{type}',
			    },
			},
		    ],
		},
		{
		    xtype: 'container',
		    layout: {
			type: 'vbox',
		    },
		    padding: '0 0 0 10',
		    items: [
			{
			    xtype: 'displayfield',
			    fieldLabel: gettext('Created'),
			    renderer: v => Proxmox.Utils.render_timestamp(v),
			    cbind: {
				value: '{created}',
			    },
			},
			{
			    xtype: 'textfield',
			    fieldLabel: gettext('Description'),
			    cbind: {
				value: '{description}',
			    },
			    emptyText: Proxmox.Utils.NoneText,
			    submitValue: false,
			    editable: false,
			},
		    ],
		},
	    ],
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
	    padding: '10 0 0 0',
	    cbind: {
		emptyText: () =>
		    Ext.String.format(gettext("Confirm your ({0}) password"), Proxmox.UserName),
	    },
	},
    ],
});
