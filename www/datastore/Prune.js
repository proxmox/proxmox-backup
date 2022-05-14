Ext.define('pbs-prune-list', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'backup-time',
	    type: 'date',
	    dateFormat: 'timestamp',
	},
    ],
});

Ext.define('PBS.PruneKeepInput', {
    extend: 'Proxmox.form.field.Integer',
    alias: 'widget.pbsPruneKeepInput',

    allowBlank: true,
    minValue: 1,

    listeners: {
	change: function(field, newValue, oldValue) {
	    if (newValue !== this.originalValue) {
		this.triggers.clear.setVisible(true);
	    }
	},
    },
    triggers: {
	clear: {
	    cls: 'pmx-clear-trigger',
	    weight: -1,
	    hidden: true,
	    handler: function() {
		this.triggers.clear.setVisible(false);
		this.setValue(this.originalValue);
	    },
	},
    },

});

Ext.define('PBS.Datastore.PruneInputPanel', {
    extend: 'Proxmox.panel.InputPanel',
    alias: 'widget.pbsDataStorePruneInputPanel',
    mixins: ['Proxmox.Mixin.CBind'],

    onGetValues: function(values) {
	var me = this;

	values["backup-type"] = me.backup_type;
	values["backup-id"] = me.backup_id;
	if (me.ns && me.ns !== '') {
	    values.ns = me.ns;
	}
	return values;
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.url) {
		throw "no url specified";
	    }
	    if (!view.backup_type) {
		throw "no backup_type specified";
	    }
	    if (!view.backup_id) {
		throw "no backup_id specified";
	    }

	    this.reload(); // initial load
	},

	reload: function() {
	    var view = this.getView();

	    // helper to allow showing why a backup is kept
	    let addKeepReasons = function(backups, params) {
		const rules = [
		    'keep-last',
		    'keep-hourly',
		    'keep-daily',
		    'keep-weekly',
		    'keep-monthly',
		    'keep-yearly',
		    'keep-all', // when all keep options are not set
		];
		let counter = {};

		backups.sort(function(a, b) {
		    return b["backup-time"] - a["backup-time"];
		});

		let ruleIndex = -1;
		let nextRule = function() {
		    let rule;
		    do {
			ruleIndex++;
			rule = rules[ruleIndex];
		    } while (!params[rule] && rule !== 'keep-all');
		    counter[rule] = 0;
		    return rule;
		};

		let rule = nextRule();
		for (let backup of backups) {
		    if (backup.keep) {
			if (backup.protected) {
			    backup.keepReason = 'protected';
			    continue;
			}
			counter[rule]++;
			if (rule !== 'keep-all') {
			    backup.keepReason = rule + ': ' + counter[rule];
			    if (counter[rule] >= params[rule]) {
				rule = nextRule();
			    }
			} else {
			    backup.keepReason = rule;
			}
		    }
		}
	    };

	    let params = view.getValues();
	    params["dry-run"] = true;
	    if (view.ns && view.ns !== '') {
		params.ns = view.ns;
	    }

	    Proxmox.Utils.API2Request({
		url: view.url,
		method: "POST",
		params: params,
		callback: function() {
		     // for easy breakpoint setting
		},
		failure: response => Ext.Msg.alert(gettext('Error'), response.htmlStatus),
		success: function(response, options) {
		    let data = response.result.data;
		    addKeepReasons(data, params);
		    view.prune_store.setData(data);
		},
	    });
	},

	control: {
	    field: { change: 'reload' },
	},
    },

    column1: [
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-last',
	    fieldLabel: gettext('keep-last'),
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-hourly',
	    fieldLabel: gettext('keep-hourly'),
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-daily',
	    fieldLabel: gettext('keep-daily'),
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-weekly',
	    fieldLabel: gettext('keep-weekly'),
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-monthly',
	    fieldLabel: gettext('keep-monthly'),
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-yearly',
	    fieldLabel: gettext('keep-yearly'),
	},
    ],


    initComponent: function() {
        var me = this;

	me.prune_store = Ext.create('Ext.data.Store', {
	    model: 'pbs-prune-list',
	    sorters: { property: 'backup-time', direction: 'DESC' },
	});

	me.column2 = [
	    {
		xtype: 'grid',
		height: 200,
		store: me.prune_store,
		columns: [
		    {
			header: gettext('Backup Time'),
			sortable: true,
			dataIndex: 'backup-time',
			renderer: function(value, metaData, record) {
			    let text = Ext.Date.format(value, 'Y-m-d H:i:s');
			    if (record.data.keep) {
				return text;
			    } else {
				return '<div style="text-decoration: line-through;">'+ text +'</div>';
			    }
			},
			flex: 1,
		    },
		    {
			text: 'Keep (reason)',
			dataIndex: 'keep',
			renderer: function(value, metaData, record) {
			    if (record.data.keep) {
				return 'true (' + record.data.keepReason + ')';
			    } else {
				return 'false';
			    }
			},
			flex: 1,
		    },
		],
	    },
	];

	me.callParent();
    },
});

Ext.define('PBS.DataStorePrune', {
    extend: 'Proxmox.window.Edit',

    onlineHelp: 'maintenance_pruning',

    method: 'POST',
    submitText: "Prune",

    isCreate: true,

    fieldDefaults: { labelWidth: 130 },

    initComponent: function() {
        var me = this;

	if (!me.datastore) {
	    throw "no datastore specified";
	}
	if (!me.backup_type) {
	    throw "no backup_type specified";
	}
	if (!me.backup_id) {
	    throw "no backup_id specified";
	}

	let ns = me.ns && me.ns !== '' ? `${me.ns} ` : '';

	Ext.apply(me, {
	    url: '/api2/extjs/admin/datastore/' + me.datastore + "/prune",
	    title: `Prune Group '${me.backup_type}/${me.backup_id}' on '${me.datastore}:${ns}'`,
	    items: [{
		xtype: 'pbsDataStorePruneInputPanel',
		url: '/api2/extjs/admin/datastore/' + me.datastore + "/prune",
		ns: me.ns,
		backup_type: me.backup_type,
		backup_id: me.backup_id,
	    }],
	});

	me.callParent();
    },
});
