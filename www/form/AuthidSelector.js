Ext.define('pbs-authids', {
    extend: 'Ext.data.Model',
    fields: [
	'authid', 'comment', 'type',
    ],
    idProperty: 'authid',
});

Ext.define('PBS.form.AuthidSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsAuthidSelector',

    allowBlank: false,
    autoSelect: false,
    valueField: 'authid',
    displayField: 'authid',

    editable: true,
    anyMatch: true,
    forceSelection: true,

    store: {
	model: 'pbs-authids',
	params: {
	    enabled: 1,
	},
	sorters: 'authid',
    },

    initComponent: function() {
	let me = this;
	me.userStore = Ext.create('Ext.data.Store', {
	    model: 'pbs-users-with-tokens',
	});
	me.userStore.on('load', this.onLoad, this);
	me.userStore.load();

	me.callParent();
    },

    onLoad: function(store, data, success) {
	let me = this;
	if (!success) return;

	let records = [];
	for (const rec of data) {
	    records.push({
		authid: rec.data.userid,
		comment: rec.data.comment,
		type: 'u',
	    });
	    let tokens = rec.data.tokens || [];
	    for (const token of tokens) {
		records.push({
		    authid: token.tokenid,
		    comment: token.comment,
		    type: 't',
		});
	    }
	}

	me.store.loadData(records);
	// we need to re-set the value, ExtJS doesn't knows that we injected data into the store
	me.setValue(me.value);
	me.validate();
    },

    listConfig: {
	width: 500,
	columns: [
	    {
		header: gettext('Type'),
		sortable: true,
		dataIndex: 'type',
		renderer: function(value) {
		    switch (value) {
			case 'u': return gettext('User');
			case 't': return gettext('API Token');
			default: return Proxmox.Utils.unknownText;
		    }
		},
		width: 80,
	    },
	    {
		header: gettext('Auth ID'),
		sortable: true,
		dataIndex: 'authid',
		renderer: Ext.String.htmlEncode,
		flex: 2,
	    },
	    {
		header: gettext('Comment'),
		sortable: false,
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 3,
	    },
	],
    },
});

