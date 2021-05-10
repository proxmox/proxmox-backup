// avoid errors when running without development tools
if (!Ext.isDefined(Ext.global.console)) {
    var console = {
        dir: function() {},
        log: function() {}
    };
}

Ext.onReady(function() {

    Ext.define('pve-param-schema', {
        extend: 'Ext.data.Model',
        fields:  [
	    'name', 'type', 'typetext', 'description', 'verbose_description',
	    'enum', 'minimum', 'maximum', 'minLength', 'maxLength',
	    'pattern', 'title', 'requires', 'format', 'default',
	    'disallow', 'extends', 'links',
	    {
		name: 'optional',
		type: 'boolean'
	    }
	]
    });

    var store = Ext.define('pve-updated-treestore', {
	extend: 'Ext.data.TreeStore',
	model: Ext.define('pve-api-doc', {
            extend: 'Ext.data.Model',
            fields:  [
		'path', 'info', 'text',
	    ]
	}),
        proxy: {
            type: 'memory',
            data: pbsapi
        },
        sorters: [{
            property: 'leaf',
            direction: 'ASC'
        }, {
            property: 'text',
            direction: 'ASC'
	}],
	filterer: 'bottomup',
	doFilter: function(node) {
	    this.filterNodes(node, this.getFilters().getFilterFn(), true);
	},

	filterNodes: function(node, filterFn, parentVisible) {
	    var me = this,
		bottomUpFiltering = me.filterer === 'bottomup',
		match = filterFn(node) && parentVisible || (node.isRoot() && !me.getRootVisible()),
		childNodes = node.childNodes,
		len = childNodes && childNodes.length, i, matchingChildren;

	    if (len) {
		for (i = 0; i < len; ++i) {
		    matchingChildren = me.filterNodes(childNodes[i], filterFn, match || bottomUpFiltering) || matchingChildren;
		}
		if (bottomUpFiltering) {
		    match = matchingChildren || match;
		}
	    }

	    node.set("visible", match, me._silentOptions);
	    return match;
	},

    }).create();

    var render_description = function(value, metaData, record) {
	var pdef = record.data;

	value = pdef.verbose_description || value;

	// TODO: try to render asciidoc correctly

	metaData.style = 'white-space:pre-wrap;'

	return Ext.htmlEncode(value);
    };

    var render_type = function(value, metaData, record) {
	var pdef = record.data;

	return pdef['enum'] ? 'enum' : (pdef.type || 'string');
    };

    let render_simple_format = function(pdef, type_fallback) {
	if (pdef.typetext)
	    return pdef.typetext;

	if (pdef['enum'])
	    return pdef['enum'].join(' | ');

	if (pdef.format)
	    return pdef.format;

	if (pdef.pattern)
	    return pdef.pattern;

	if (pdef.type === 'boolean')
	    return `<true|false>`;

	if (type_fallback && pdef.type)
	    return `<${pdef.type}>`;

	return;
    };

    let render_format = function(value, metaData, record) {
	let pdef = record.data;

	metaData.style = 'white-space:normal;'

	if (pdef.type === 'array' && pdef.items) {
	    let format = render_simple_format(pdef.items, true);
	    return `[${Ext.htmlEncode(format)}, ...]`;
	}

	return Ext.htmlEncode(render_simple_format(pdef) || '');
    };

    var real_path = function(path) {
	return path.replace(/^.*\/_upgrade_(\/)?/, "/");
    };

    var permission_text = function(permission) {
	let permhtml = "";

	if (permission.user) {
	    if (!permission.description) {
		if (permission.user === 'world') {
		    permhtml += "Accessible without any authentication.";
		} else if (permission.user === 'all') {
		    permhtml += "Accessible by all authenticated users.";
		} else {
		    permhtml += 'Onyl accessible by user "' +
			permission.user + '"';
		}
	    }
	} else if (permission.check) {
	    permhtml += "<pre>Check: " +
		Ext.htmlEncode(Ext.JSON.encode(permission.check))  + "</pre>";
	} else if (permission.userParam) {
	    permhtml += `<div>Check if user matches parameter '${permission.userParam}'`;
	} else if (permission.or) {
	    permhtml += "<div>Or<div style='padding-left: 10px;'>";
	    Ext.Array.each(permission.or, function(sub_permission) {
		permhtml += permission_text(sub_permission);
	    })
	    permhtml += "</div></div>";
	} else if (permission.and) {
	    permhtml += "<div>And<div style='padding-left: 10px;'>";
	    Ext.Array.each(permission.and, function(sub_permission) {
		permhtml += permission_text(sub_permission);
	    })
	    permhtml += "</div></div>";
	} else {
	    //console.log(permission);
	    permhtml += "Unknown syntax!";
	}

	return permhtml;
    };

    var render_docu = function(data) {
	var md = data.info;

	// console.dir(data);

	var items = [];

	var clicmdhash = {
	    GET: 'get',
	    POST: 'create',
	    PUT: 'set',
	    DELETE: 'delete'
	};

	Ext.Array.each(['GET', 'POST', 'PUT', 'DELETE'], function(method) {
	    var info = md[method];
	    if (info) {

		var usage = "";

		usage += "<table><tr><td>HTTP:&nbsp;&nbsp;&nbsp;</td><td>"
		    + method + " " + real_path("/api2/json" + data.path) + "</td></tr>";

		var sections = [
		    {
			title: 'Description',
			html: Ext.htmlEncode(info.description),
			bodyPadding: 10
		    },
		    {
			title: 'Usage',
			html: usage,
			bodyPadding: 10
		    }
		];

		if (info.parameters && info.parameters.properties) {

		    var pstore = Ext.create('Ext.data.Store', {
			model: 'pve-param-schema',
			proxy: {
			    type: 'memory'
			},
			groupField: 'optional',
			sorters: [
			    {
				property: 'name',
				direction: 'ASC'
			    }
			]
		    });

		    Ext.Object.each(info.parameters.properties, function(name, pdef) {
			pdef.name = name;
			pstore.add(pdef);
		    });

		    pstore.sort();

		    var groupingFeature = Ext.create('Ext.grid.feature.Grouping',{
			enableGroupingMenu: false,
			groupHeaderTpl: '<tpl if="groupValue">Optional</tpl><tpl if="!groupValue">Required</tpl>'
		    });

		    sections.push({
			xtype: 'gridpanel',
			title: 'Parameters',
			features: [groupingFeature],
			store: pstore,
			viewConfig: {
			    trackOver: false,
			    stripeRows: true
			},
			columns: [
			    {
				header: 'Name',
				dataIndex: 'name',
				flex: 1
			    },
			    {
				header: 'Type',
				dataIndex: 'type',
				renderer: render_type,
				flex: 1
			    },
			    {
				header: 'Default',
				dataIndex: 'default',
				flex: 1
			    },
			    {
				header: 'Format',
				dataIndex: 'type',
				renderer: render_format,
				flex: 2
			    },
			    {
				header: 'Description',
				dataIndex: 'description',
				renderer: render_description,
				flex: 6
			    }
			]
		    });

		}

		if (info.returns) {

		    var retinf = info.returns;
		    var rtype = retinf.type;
		    if (!rtype && retinf.items)
			rtype = 'array';
		    if (!rtype)
			rtype = 'object';

		    var rpstore = Ext.create('Ext.data.Store', {
			model: 'pve-param-schema',
			proxy: {
			    type: 'memory'
			},
			groupField: 'optional',
			sorters: [
			    {
				property: 'name',
				direction: 'ASC'
			   }
			]
		    });

		    var properties;
		    if (rtype === 'array' && retinf.items.properties) {
			properties = retinf.items.properties;
		    }

		    if (rtype === 'object' && retinf.properties) {
			properties = retinf.properties;
		    }

		    Ext.Object.each(properties, function(name, pdef) {
			pdef.name = name;
			rpstore.add(pdef);
		    });

		    rpstore.sort();

		    var groupingFeature = Ext.create('Ext.grid.feature.Grouping',{
			enableGroupingMenu: false,
			groupHeaderTpl: '<tpl if="groupValue">Optional</tpl><tpl if="!groupValue">Obligatory</tpl>'
		    });
		    var returnhtml;
		    if (retinf.items) {
			returnhtml = '<pre>items: ' + Ext.htmlEncode(JSON.stringify(retinf.items, null, 4)) + '</pre>';
		    }

		    if (retinf.properties) {
			returnhtml = returnhtml || '';
			returnhtml += '<pre>properties:' + Ext.htmlEncode(JSON.stringify(retinf.properties, null, 4)) + '</pre>';
		    }

		    var rawSection = Ext.create('Ext.panel.Panel', {
			bodyPadding: '0px 10px 10px 10px',
			html: returnhtml,
			hidden: true
		    });

		    sections.push({
			xtype: 'gridpanel',
			title: 'Returns: ' + rtype,
			features: [groupingFeature],
			store: rpstore,
			viewConfig: {
			    trackOver: false,
			    stripeRows: true
			},
		    columns: [
			{
			    header: 'Name',
			    dataIndex: 'name',
			    flex: 1
			},
			{
			    header: 'Type',
			    dataIndex: 'type',
			    renderer: render_type,
			    flex: 1
			},
			{
			    header: 'Default',
			    dataIndex: 'default',
			    flex: 1
			},
			{
			    header: 'Format',
			    dataIndex: 'type',
			    renderer: render_format,
			    flex: 2
			},
			{
			    header: 'Description',
			    dataIndex: 'description',
			    renderer: render_description,
			    flex: 6
			}
		    ],
		    bbar: [
			{
			    xtype: 'button',
			    text: 'Show RAW',
			    handler: function(btn) {
				rawSection.setVisible(!rawSection.isVisible());
				btn.setText(rawSection.isVisible() ? 'Hide RAW' : 'Show RAW');
			    }}
		    ]
		});

		sections.push(rawSection);


		}

		if (!data.path.match(/\/_upgrade_/)) {
		    var permhtml = '';

		    if (!info.permissions) {
			permhtml = "Root only.";
		    } else {
			if (info.permissions.description) {
			    permhtml += "<div style='white-space:pre-wrap;padding-bottom:10px;'>" +
				Ext.htmlEncode(info.permissions.description) + "</div>";
			}
			permhtml += permission_text(info.permissions);
		    }

		    // we do not have this information for PBS api
		    //if (!info.allowtoken) {
		    //    permhtml += "<br />This API endpoint is not available for API tokens."
		    //}

		    sections.push({
			title: 'Required permissions',
			bodyPadding: 10,
			html: permhtml
		    });
		}

		items.push({
		    title: method,
		    autoScroll: true,
		    defaults: {
			border: false
		    },
		    items: sections
		});
	    }
	});

	var ct = Ext.getCmp('docview');
	ct.setTitle("Path: " +  real_path(data.path));
	ct.removeAll(true);
	ct.add(items);
	ct.setActiveTab(0);
    };

    Ext.define('Ext.form.SearchField', {
	extend: 'Ext.form.field.Text',
	alias: 'widget.searchfield',

	emptyText: 'Search...',

	flex: 1,

	inputType: 'search',
	listeners: {
	    'change': function(){

		var value = this.getValue();
		if (!Ext.isEmpty(value)) {
		    store.filter({
			property: 'path',
			value: value,
			anyMatch: true
		    });
		} else {
		    store.clearFilter();
		}
	    }
	}
    });

    var tree = Ext.create('Ext.tree.Panel', {
	title: 'Resource Tree',
	tbar: [
	    {
		xtype: 'searchfield',
	    }
	],
	tools: [
	    {
		type: 'expand',
		tooltip: 'Expand all',
		tooltipType: 'title',
		callback: (tree) => tree.expandAll(),
	    },
	    {
		type: 'collapse',
		tooltip: 'Collapse all',
		tooltipType: 'title',
		callback: (tree) => tree.collapseAll(),
	    },
	],
        store: store,
	width: 200,
        region: 'west',
        split: true,
        margins: '5 0 5 5',
        rootVisible: false,
	listeners: {
	    selectionchange: function(v, selections) {
		if (!selections[0])
		    return;
		var rec = selections[0];
		render_docu(rec.data);
		location.hash = '#' + rec.data.path;
	    }
	}
    });

    Ext.create('Ext.container.Viewport', {
	layout: 'border',
	renderTo: Ext.getBody(),
	items: [
	    tree,
	    {
		xtype: 'tabpanel',
		title: 'Documentation',
		id: 'docview',
		region: 'center',
		margins: '5 5 5 0',
		layout: 'fit',
		items: []
	    }
	]
    });

    var deepLink = function() {
	var path = window.location.hash.substring(1).replace(/\/\s*$/, '')
	var endpoint = store.findNode('path', path);

	if (endpoint) {
	    tree.getSelectionModel().select(endpoint);
	    tree.expandPath(endpoint.getPath());
	    render_docu(endpoint.data);
	}
    }
    window.onhashchange = deepLink;

    deepLink();

});
